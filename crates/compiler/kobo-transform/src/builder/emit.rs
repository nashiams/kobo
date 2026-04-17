use crate::classify::{BindingMetadata, BindingState};
use kobo_ir::{
    BindingUsage, EscapeKind, HintConflictFact, NodeKind, TransformBindingFacts, UseEvent, UseKind,
};
use kobo_parser::KoboBinding;

use super::helpers::{build_binding_event_node, build_decl_node, build_scope_node};
use super::{PendingHint, TransformFactsBuilder};

impl TransformFactsBuilder<'_> {
    pub(super) fn emit_declared_binding(
        &mut self,
        ident: &syn::Ident,
        init: Option<&syn::Expr>,
        hint: Option<PendingHint>,
    ) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span).cloned() else {
            return;
        };

        let binding_state = self.emit_binding(&binding, init, hint);
        if let Some(init_expr) = init {
            self.walk_local_initializer(init_expr);
        }
        self.ctx.define(&binding.ident, binding_state);
    }

    pub(super) fn emit_pat_binding(
        &mut self,
        pat: &syn::Pat,
        init: Option<&syn::Expr>,
        hint: Option<PendingHint>,
    ) {
        let Some(binding) = self.binding_for_pat(pat).cloned() else {
            return;
        };

        let binding_state = self.emit_binding(&binding, init, hint);
        self.ctx.define(&binding.ident, binding_state);
    }

    pub(super) fn emit_binding(
        &mut self,
        binding: &KoboBinding,
        init: Option<&syn::Expr>,
        hint: Option<PendingHint>,
    ) -> BindingState {
        let metadata = self.binding_metadata(binding, init);
        let decl_id = self.id_gen.next_kir_id();
        self.nodes.push(build_decl_node(binding, decl_id, metadata));
        self.push_binding_fact(binding, decl_id, metadata, hint);

        BindingState {
            decl_id,
            kind: binding.kind,
            resource_kind: metadata.resource_kind,
            is_copy_known: metadata.is_copy_known,
            is_generic: metadata.is_generic,
            small_clone_profile: metadata.small_clone_profile,
        }
    }

    fn push_binding_fact(
        &mut self,
        binding: &KoboBinding,
        decl_id: kobo_ir::KirNodeId,
        metadata: BindingMetadata,
        hint: Option<PendingHint>,
    ) {
        let box_reason = self.detect_box_reason(binding);
        let shared_facts = kobo_ir::SharedBindingFacts {
            box_reason,
            ..Default::default()
        };
        self.fact_indices
            .insert(decl_id, self.transform_facts.bindings.len());
        self.transform_facts.bindings.push(TransformBindingFacts {
            node: decl_id,
            ast_id: binding.id,
            binding_name: binding.ident.to_string(),
            span: binding.span,
            resource_kind: metadata.resource_kind,
            hint: hint.map(|pending| pending.hint),
            hint_span: hint.map(|pending| pending.span),
            is_copy_known: metadata.is_copy_known,
            is_generic: metadata.is_generic,
            is_async: self
                .function_stack
                .last()
                .is_some_and(|frame| frame.is_async),
            usage: BindingUsage::new(binding.span),
            shared_facts,
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
        });
    }

    pub(super) fn enter_scope(&mut self, span: kobo_ir::KoboSpan) {
        self.nodes.push(build_scope_node(
            self.id_gen.next_kir_id(),
            NodeKind::ScopeStart,
            span,
        ));
    }

    pub(super) fn exit_scope(&mut self, span: kobo_ir::KoboSpan) {
        self.nodes.push(build_scope_node(
            self.id_gen.next_kir_id(),
            NodeKind::ScopeEnd,
            span,
        ));
    }

    pub(super) fn emit_move(&mut self, expr: &syn::Expr, escape_kind: Option<EscapeKind>) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Move,
            span,
            binding_state.decl_id,
        ));
        self.record_event(binding_state.decl_id, UseEvent::Moved { span });
        if let Some(kind) = escape_kind.filter(|kind| {
            !matches!(
                (kind, binding_state.kind),
                (
                    EscapeKind::ReturnedFromFunction,
                    kobo_parser::KoboBindingKind::Parameter
                        | kobo_parser::KoboBindingKind::SelfParam
                        | kobo_parser::KoboBindingKind::MutSelfParam
                )
            )
        }) {
            self.record_event(binding_state.decl_id, UseEvent::Escaped { kind, span });
        }
        true
    }

    pub(super) fn emit_use(&mut self, expr: &syn::Expr, use_kind: UseKind) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Use(use_kind),
            span,
            binding_state.decl_id,
        ));
        let event = match use_kind {
            UseKind::Read => UseEvent::ReadOnly { span },
            UseKind::Write => UseEvent::Mutated { span },
        };
        self.record_event(binding_state.decl_id, event);
        true
    }

    pub(super) fn emit_borrow(
        &mut self,
        expr: &syn::Expr,
        borrow_kind: kobo_ir::BorrowKind,
    ) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Borrow(borrow_kind),
            span,
            binding_state.decl_id,
        ));
        self.record_event(
            binding_state.decl_id,
            UseEvent::Borrowed {
                kind: borrow_kind,
                span,
            },
        );
        true
    }

    pub(super) fn emit_ephemeral_borrow(
        &mut self,
        expr: &syn::Expr,
        borrow_kind: kobo_ir::BorrowKind,
    ) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        let use_kind = match borrow_kind {
            kobo_ir::BorrowKind::Immutable => UseKind::Read,
            kobo_ir::BorrowKind::Mutable => UseKind::Write,
        };
        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Use(use_kind),
            span,
            binding_state.decl_id,
        ));
        self.record_event(
            binding_state.decl_id,
            UseEvent::Borrowed {
                kind: borrow_kind,
                span,
            },
        );
        true
    }

    pub(super) fn record_event(&mut self, decl_id: kobo_ir::KirNodeId, event: UseEvent) {
        let Some(index) = self.fact_indices.get(&decl_id).copied() else {
            return;
        };
        self.transform_facts.bindings[index].usage.push_use(event);
    }

    #[allow(dead_code)]
    pub(super) fn record_hint_conflict(&mut self, conflict: HintConflictFact) {
        self.hint_conflicts.push(conflict);
    }

    pub(super) fn push_borrow_alias(&mut self, alias: crate::escape::BorrowAlias) {
        self.borrow_aliases.push(alias);
    }

    pub(super) fn push_move_alias(&mut self, alias: crate::escape::MoveAlias) {
        self.move_aliases.push(alias);
    }

    pub(super) fn record_elision_candidate(&mut self, candidate: kobo_ir::CloneElisionCandidate) {
        self.clone_elision_candidates.push(candidate);
    }

    pub(super) fn set_plain_clone_alias(
        &mut self,
        decl_id: kobo_ir::KirNodeId,
        source: kobo_ir::KirNodeId,
        move_span: kobo_ir::KoboSpan,
    ) {
        let Some(index) = self.fact_indices.get(&decl_id).copied() else {
            return;
        };
        let binding = &mut self.transform_facts.bindings[index];
        binding.plain_clone_alias = true;
        binding.plain_clone_source = Some(source);
        binding.plain_clone_move_span = Some(move_span);
    }

    pub(super) fn set_elision_skip_reason(
        &mut self,
        decl_id: kobo_ir::KirNodeId,
        reason: kobo_ir::ElisionSkipReason,
    ) {
        let Some(index) = self.fact_indices.get(&decl_id).copied() else {
            return;
        };
        self.transform_facts.bindings[index].elision_skip_reason = Some(reason);
    }
}
