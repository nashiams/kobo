use kobo_ir::{KirNode, NodeKind};

use crate::liveness::{BindingId, BindingTable, LivenessState};
use crate::ownership_facts::MoveFact;

pub(crate) fn visit(node: &KirNode, binding_table: &mut BindingTable, move_facts: &mut Vec<MoveFact>) {
    let Some(binding) = node.decl_id.map(BindingId) else {
        return;
    };

    match node.kind {
        NodeKind::Move => handle_move(binding, node.span, binding_table, move_facts),
        NodeKind::Use(_) | NodeKind::Borrow(_) => handle_use_like(binding, node.span, binding_table, move_facts),
        _ => {}
    }
}

fn handle_move(
    binding: BindingId,
    move_site: kobo_ir::KoboSpan,
    binding_table: &mut BindingTable,
    move_facts: &mut Vec<MoveFact>,
) {
    let Some(state) = binding_table.state(binding) else {
        return;
    };

    match state {
        LivenessState::Live => binding_table.mark_moved(binding, move_site),
        LivenessState::Moved { at } => move_facts.push(MoveFact {
            binding: binding.0,
            move_site: at,
            later_use: move_site,
        }),
        LivenessState::Borrowed { .. } => {
            move_facts.push(MoveFact {
                binding: binding.0,
                move_site,
                later_use: move_site,
            });
            binding_table.invalidate_borrows(binding);
            binding_table.mark_moved(binding, move_site);
        }
    }
}

fn handle_use_like(
    binding: BindingId,
    use_site: kobo_ir::KoboSpan,
    binding_table: &mut BindingTable,
    move_facts: &mut Vec<MoveFact>,
) {
    let Some(LivenessState::Moved { at }) = binding_table.state(binding) else {
        return;
    };

    move_facts.push(MoveFact {
        binding: binding.0,
        move_site: at,
        later_use: use_site,
    });
}
