use std::collections::HashMap;

use kobo_ir::{BorrowKind, FileSet, KirNodeId, KoboSpan};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct BindingId(pub KirNodeId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BindingRecord {
    pub(crate) id: BindingId,
    pub(crate) name: String,
    pub(crate) decl_span: KoboSpan,
    pub(crate) state: LivenessState,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum LivenessState {
    Live,
    Moved { at: KoboSpan },
    Borrowed { at: KoboSpan, kind: BorrowKind },
}

#[derive(Debug, Default)]
pub(crate) struct Scope {
    pub(crate) bindings: Vec<BindingId>,
}

#[derive(Debug, Default)]
pub(crate) struct LexicalEnv {
    scopes: Vec<Scope>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveBorrow {
    owner: BindingId,
    at: KoboSpan,
    kind: BorrowKind,
}

#[derive(Debug, Default)]
pub(crate) struct BindingTable {
    records: HashMap<BindingId, BindingRecord>,
    pending_decl: Option<BindingId>,
    borrow_targets_by_owner: HashMap<BindingId, BindingId>,
    active_borrows_by_target: HashMap<BindingId, Vec<ActiveBorrow>>,
}

impl LexicalEnv {
    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub(crate) fn pop_scope(&mut self, binding_table: &mut BindingTable) {
        let Some(scope) = self.scopes.pop() else {
            return;
        };

        for binding in scope.bindings {
            binding_table.retire_binding(binding);
        }
    }

    pub(crate) fn declare_binding(&mut self, binding_id: BindingId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.bindings.push(binding_id);
        }
    }
}

impl BindingTable {
    pub(crate) fn declare_binding(
        &mut self,
        binding_id: BindingId,
        name: String,
        decl_span: KoboSpan,
    ) {
        self.records.insert(
            binding_id,
            BindingRecord {
                id: binding_id,
                name,
                decl_span,
                state: LivenessState::Live,
            },
        );
        self.pending_decl = Some(binding_id);
    }

    pub(crate) fn state(&self, binding_id: BindingId) -> Option<LivenessState> {
        self.records.get(&binding_id).map(|record| record.state)
    }

    pub(crate) fn mark_moved(&mut self, binding_id: BindingId, at: KoboSpan) {
        self.update_state(binding_id, LivenessState::Moved { at });
    }

    pub(crate) fn mark_borrowed(
        &mut self,
        target: BindingId,
        borrow_site: KoboSpan,
        borrow_kind: BorrowKind,
    ) {
        let Some(owner) = self.pending_decl.take() else {
            return;
        };

        self.borrow_targets_by_owner.insert(owner, target);
        self.active_borrows_by_target
            .entry(target)
            .or_default()
            .push(ActiveBorrow {
                owner,
                at: borrow_site,
                kind: borrow_kind,
            });
        self.refresh_borrow_state(target);
    }

    pub(crate) fn invalidate_borrows(&mut self, binding_id: BindingId) {
        if let Some(active_borrows) = self.active_borrows_by_target.remove(&binding_id) {
            for active_borrow in active_borrows {
                self.borrow_targets_by_owner.remove(&active_borrow.owner);
            }
        }
    }

    pub(crate) fn clear_pending_decl(&mut self) {
        self.pending_decl = None;
    }

    pub(crate) fn iter_records(&self) -> impl Iterator<Item = &BindingRecord> {
        self.records.values()
    }

    fn retire_binding(&mut self, binding_id: BindingId) {
        self.pending_decl = self.pending_decl.filter(|pending| *pending != binding_id);

        let Some(target) = self.borrow_targets_by_owner.remove(&binding_id) else {
            return;
        };

        if let Some(active_borrows) = self.active_borrows_by_target.get_mut(&target) {
            active_borrows.retain(|active_borrow| active_borrow.owner != binding_id);
        }
        self.refresh_borrow_state(target);
    }

    fn refresh_borrow_state(&mut self, binding_id: BindingId) {
        let next_state = self
            .active_borrows_by_target
            .get(&binding_id)
            .and_then(|active_borrows| active_borrows.first().copied())
            .map(|active_borrow| LivenessState::Borrowed {
                at: active_borrow.at,
                kind: active_borrow.kind,
            })
            .unwrap_or(LivenessState::Live);

        if matches!(next_state, LivenessState::Live)
            && self
                .active_borrows_by_target
                .get(&binding_id)
                .is_some_and(Vec::is_empty)
        {
            self.active_borrows_by_target.remove(&binding_id);
        }

        self.update_state(binding_id, next_state);
    }

    fn update_state(&mut self, binding_id: BindingId, next_state: LivenessState) {
        if let Some(record) = self.records.get_mut(&binding_id) {
            record.state = next_state;
        }
    }
}

pub(crate) fn binding_name(file_set: &FileSet, span: KoboSpan) -> String {
    file_set
        .get(span.file_id)
        .and_then(|file| file.snippet(span))
        .unwrap_or("<unknown>")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileSetBuilder, KirNodeId};

    use super::{BindingId, BindingTable, LexicalEnv, LivenessState};

    #[test]
    fn shadowed_same_scope_bindings_keep_distinct_records() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file("demo.kobo".into(), "x\nx".to_owned());
        let mut env = LexicalEnv::default();
        let mut table = BindingTable::default();

        env.push_scope();
        let first = BindingId(KirNodeId(1));
        let second = BindingId(KirNodeId(2));
        table.declare_binding(first, "x".to_owned(), kobo_ir::KoboSpan::new(0, 1, file_id));
        env.declare_binding(first);
        table.declare_binding(second, "x".to_owned(), kobo_ir::KoboSpan::new(2, 3, file_id));
        env.declare_binding(second);

        assert_eq!(table.state(first), Some(LivenessState::Live));
        assert_eq!(table.state(second), Some(LivenessState::Live));
    }

    #[test]
    fn nested_scope_pop_does_not_touch_outer_binding() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file("demo.kobo".into(), "x\n{ x }".to_owned());
        let mut env = LexicalEnv::default();
        let mut table = BindingTable::default();

        let outer = BindingId(KirNodeId(1));
        let inner = BindingId(KirNodeId(2));

        env.push_scope();
        table.declare_binding(outer, "x".to_owned(), kobo_ir::KoboSpan::new(0, 1, file_id));
        env.declare_binding(outer);
        env.push_scope();
        table.declare_binding(inner, "x".to_owned(), kobo_ir::KoboSpan::new(5, 6, file_id));
        env.declare_binding(inner);
        env.pop_scope(&mut table);

        assert_eq!(table.state(outer), Some(LivenessState::Live));
        assert_eq!(table.state(inner), Some(LivenessState::Live));
    }
}
