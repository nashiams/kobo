use std::collections::{HashMap, HashSet};

use kobo_ir::{BorrowKind, CloneElisionCandidate, TransformFacts, UseEvent};

use crate::clone_elision::decide_clone_elision;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BorrowAlias {
    pub alias: kobo_ir::KirNodeId,
    pub source: kobo_ir::KirNodeId,
    pub kind: BorrowKind,
    pub span: kobo_ir::KoboSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MoveAlias {
    pub alias: kobo_ir::KirNodeId,
    pub source: kobo_ir::KirNodeId,
    pub span: kobo_ir::KoboSpan,
    pub plain_clone_alias: bool,
}

pub(crate) fn finalize_transform_facts(
    facts: &mut TransformFacts,
    borrow_aliases: &[BorrowAlias],
    move_aliases: &[MoveAlias],
    clone_elision_candidates: &[CloneElisionCandidate],
) {
    materialize_borrow_alias_uses(facts, borrow_aliases);
    materialize_move_alias_uses(facts, move_aliases, clone_elision_candidates);

    for binding in &mut facts.bindings {
        binding.usage.sort_uses();
        let mut shared_facts = kobo_ir::derive_shared_facts(&binding.usage, None);
        shared_facts.node_id = binding.node;
        shared_facts.box_reason = binding.shared_facts.box_reason;
        shared_facts.needs_send = binding.shared_facts.needs_send;
        if shared_facts.needs_sharing || shared_facts.needs_mutable_wrapper {
            shared_facts.box_reason = None;
        }

        let move_candidate = clone_elision_candidates
            .iter()
            .find(|candidate| candidate.source == binding.node);
        let (clone_elision, elision_fallback) =
            decide_clone_elision(move_candidate, &binding.usage, &shared_facts);
        shared_facts = kobo_ir::derive_shared_facts(&binding.usage, clone_elision.as_ref());
        shared_facts.node_id = binding.node;
        shared_facts.box_reason = binding.shared_facts.box_reason;
        shared_facts.needs_send = binding.shared_facts.needs_send;
        if shared_facts.needs_sharing || shared_facts.needs_mutable_wrapper {
            shared_facts.box_reason = None;
        }

        binding.shared_facts = shared_facts;
        binding.clone_elision = clone_elision;
        binding.elision_fallback = elision_fallback;
    }

    facts.sync_views();
}

fn materialize_borrow_alias_uses(facts: &mut TransformFacts, borrow_aliases: &[BorrowAlias]) {
    let alias_usage_by_node = facts
        .bindings
        .iter()
        .map(|binding| (binding.node, binding.usage.clone()))
        .collect::<HashMap<_, _>>();

    for binding in &mut facts.bindings {
        let mut synthetic_events = Vec::new();
        let mut seen_spans = HashSet::new();

        for alias in borrow_aliases
            .iter()
            .filter(|alias| alias.source == binding.node)
        {
            let Some(alias_usage) = alias_usage_by_node.get(&alias.alias) else {
                continue;
            };
            for event in &alias_usage.uses {
                if !seen_spans.insert(event.span()) {
                    continue;
                }
                let materialized = materialize_alias_event(*alias, event);
                synthetic_events.push(materialized);
            }
        }

        binding.usage.uses.extend(synthetic_events);
    }
}

fn materialize_move_alias_uses(
    facts: &mut TransformFacts,
    move_aliases: &[MoveAlias],
    clone_elision_candidates: &[CloneElisionCandidate],
) {
    let alias_usage_by_node = facts
        .bindings
        .iter()
        .map(|binding| (binding.node, binding.usage.clone()))
        .collect::<HashMap<_, _>>();
    let candidate_pairs = clone_elision_candidates
        .iter()
        .map(|candidate| (candidate.source, candidate.alias))
        .collect::<HashSet<_>>();

    for binding in &mut facts.bindings {
        let mut synthetic_events = Vec::new();
        let mut seen_spans = binding
            .usage
            .uses
            .iter()
            .map(UseEvent::span)
            .collect::<HashSet<_>>();

        for alias in move_aliases
            .iter()
            .filter(|alias| alias.source == binding.node)
        {
            if alias.plain_clone_alias || candidate_pairs.contains(&(alias.source, alias.alias)) {
                continue;
            }

            if seen_spans.insert(alias.span) {
                synthetic_events.push(UseEvent::ReadOnly { span: alias.span });
            }

            let Some(alias_usage) = alias_usage_by_node.get(&alias.alias) else {
                continue;
            };
            for event in &alias_usage.uses {
                let materialized = materialize_move_alias_event(event);
                if !seen_spans.insert(materialized.span()) {
                    continue;
                }
                synthetic_events.push(materialized);
            }
        }

        binding.usage.uses.extend(synthetic_events);
    }
}

fn materialize_alias_event(alias: BorrowAlias, event: &UseEvent) -> UseEvent {
    match alias.kind {
        BorrowKind::Immutable => UseEvent::ReadOnly { span: event.span() },
        BorrowKind::Mutable => UseEvent::Mutated { span: event.span() },
    }
}

fn materialize_move_alias_event(event: &UseEvent) -> UseEvent {
    match event {
        UseEvent::Mutated { span }
        | UseEvent::Borrowed {
            kind: BorrowKind::Mutable,
            span,
        } => UseEvent::Mutated { span: *span },
        UseEvent::ReadOnly { span }
        | UseEvent::Moved { span }
        | UseEvent::Escaped { span, .. }
        | UseEvent::Borrowed {
            kind: BorrowKind::Immutable,
            span,
        } => UseEvent::ReadOnly { span: *span },
    }
}
