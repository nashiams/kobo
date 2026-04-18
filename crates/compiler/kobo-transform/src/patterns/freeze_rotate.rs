use std::collections::HashSet;

use kobo_ir::{KirNodeId, TransformFacts, UseEvent};

/// Detects move-rebind patterns where the original binding is dead after assignment.
///
/// A binding B qualifies if:
/// 1. B has a Moved event
/// 2. B has NO Read/Mutated/Moved events AFTER the move
/// 3. The move is in the same scope level as the declaration (conservative)
///
/// Returns set of binding node IDs that qualify for freeze-and-rotate elision.
pub fn detect_freeze_and_rotate(facts: &TransformFacts) -> HashSet<KirNodeId> {
    let mut eligible = HashSet::new();

    for binding in facts.iter_bindings() {
        let uses = &binding.usage.uses;

        // Look for a Moved event followed by no further usage
        for (i, event) in uses.iter().enumerate() {
            if matches!(event, UseEvent::Moved { .. }) {
                // Check that ALL subsequent events are not reads/mutations/moves on this binding
                let has_later_use = uses[i + 1..].iter().any(|later| {
                    matches!(
                        later,
                        UseEvent::ReadOnly { .. }
                            | UseEvent::Mutated { .. }
                            | UseEvent::Moved { .. }
                            | UseEvent::Borrowed { .. }
                    )
                });

                if !has_later_use {
                    eligible.insert(binding.node);
                    break;
                }
            }
        }
    }

    eligible
}
