use kobo_ir::{BorrowKind, KirNode, NodeKind, UseKind};

use crate::liveness::{BindingId, BindingTable, LivenessState};
use crate::ownership_facts::BorrowFact;

pub(crate) fn visit(
    node: &KirNode,
    binding_table: &mut BindingTable,
    borrow_facts: &mut Vec<BorrowFact>,
) {
    let Some(binding) = node.decl_id.map(BindingId) else {
        return;
    };

    match node.kind {
        NodeKind::Borrow(borrow_kind) => handle_borrow(node, binding, borrow_kind, binding_table, borrow_facts),
        NodeKind::Use(UseKind::Write) => handle_write(node, binding, binding_table, borrow_facts),
        _ => {}
    }
}

fn handle_borrow(
    node: &KirNode,
    binding: BindingId,
    borrow_kind: BorrowKind,
    binding_table: &mut BindingTable,
    borrow_facts: &mut Vec<BorrowFact>,
) {
    let Some(state) = binding_table.state(binding) else {
        return;
    };

    match state {
        LivenessState::Moved { .. } => {}
        LivenessState::Borrowed { at, kind } if borrow_kind == BorrowKind::Mutable => {
            borrow_facts.push(BorrowFact {
                binding: binding.0,
                borrow_site: at,
                conflict_site: node.span,
                borrow_kind: kind,
            });
            binding_table.clear_pending_decl();
        }
        LivenessState::Live | LivenessState::Borrowed { .. } => {
            binding_table.mark_borrowed(binding, node.span, borrow_kind);
        }
    }
}

fn handle_write(
    node: &KirNode,
    binding: BindingId,
    binding_table: &mut BindingTable,
    borrow_facts: &mut Vec<BorrowFact>,
) {
    let Some(state) = binding_table.state(binding) else {
        return;
    };

    if let LivenessState::Borrowed { at, kind } = state {
        borrow_facts.push(BorrowFact {
            binding: binding.0,
            borrow_site: at,
            conflict_site: node.span,
            borrow_kind: kind,
        });
    }
}
