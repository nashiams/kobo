use kobo_ir::{
    BindingUsage, CloneElisionDecision, ElisionFallbackReason, KoboSpan, SharedBindingFacts,
    UseEvent,
};

pub(crate) fn first_move_span(usage: &BindingUsage) -> Option<KoboSpan> {
    usage.uses.iter().find_map(|event| match event {
        UseEvent::Moved { span } => Some(*span),
        _ => None,
    })
}

pub(crate) fn has_use_after(usage: &BindingUsage, span: KoboSpan) -> bool {
    usage
        .uses
        .iter()
        .any(|event| event.span().start > span.start)
}

pub(crate) fn decide_clone_elision(
    usage: &BindingUsage,
    shared_facts: &SharedBindingFacts,
) -> (Option<CloneElisionDecision>, Option<ElisionFallbackReason>) {
    let Some(move_span) = first_move_span(usage) else {
        return (None, None);
    };

    let no_uses_after_move = !has_use_after(usage, move_span);
    let live_borrow_at_move = shared_facts.live_borrow_at_move;

    if !no_uses_after_move || live_borrow_at_move {
        return (
            Some(CloneElisionDecision::Clone),
            Some(ElisionFallbackReason::MoveSafetyCheckFailed),
        );
    }

    if cfg!(debug_assertions) {
        debug_assert!(
            no_uses_after_move,
            "CloneElisionDecision::Move issued but original binding has uses after move site"
        );
        debug_assert!(
            !live_borrow_at_move,
            "CloneElisionDecision::Move issued but a live borrow exists at the move site"
        );
    }

    (Some(CloneElisionDecision::Move), None)
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, KoboSpan};

    use super::*;

    fn span(start: u32) -> KoboSpan {
        KoboSpan::new(start, start + 1, FileId(0))
    }

    #[test]
    fn dead_original_assignment_uses_move() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![UseEvent::Moved { span: span(10) }],
        };
        let shared_facts = SharedBindingFacts::default();

        let decision = decide_clone_elision(&usage, &shared_facts);

        assert_eq!(decision, (Some(CloneElisionDecision::Move), None));
    }

    #[test]
    fn later_uses_force_conservative_clone() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![
                UseEvent::Moved { span: span(10) },
                UseEvent::ReadOnly { span: span(20) },
            ],
        };
        let shared_facts = SharedBindingFacts::default();

        let decision = decide_clone_elision(&usage, &shared_facts);

        assert_eq!(
            decision,
            (
                Some(CloneElisionDecision::Clone),
                Some(ElisionFallbackReason::MoveSafetyCheckFailed),
            )
        );
    }

    #[test]
    fn live_borrow_at_move_forces_conservative_clone() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![UseEvent::Moved { span: span(10) }],
        };
        let shared_facts = SharedBindingFacts {
            live_borrow_at_move: true,
            ..SharedBindingFacts::default()
        };

        let decision = decide_clone_elision(&usage, &shared_facts);

        assert_eq!(
            decision,
            (
                Some(CloneElisionDecision::Clone),
                Some(ElisionFallbackReason::MoveSafetyCheckFailed),
            )
        );
    }
}
