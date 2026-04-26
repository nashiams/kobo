//! Phase 11: Human-readable explanations for solver decisions.
//!
//! Generates natural-language explanations that can be shown in CLI
//! output or injected as source comments.

use kobo_ir::OwnershipTier;

use crate::decision_class::{ClassifiedDecision, Confidence, DecisionClass};

/// Generate a human-readable explanation for a decision.
pub fn explain_decision(decision: &ClassifiedDecision) -> String {
    let tier_desc = explain_tier(decision.tier);
    let class_desc = match decision.class {
        DecisionClass::Greedy => "decided by the greedy pass (simple analysis)",
        DecisionClass::LatticeUnique => "decided by the lattice solver (constraint propagation)",
        DecisionClass::BacktrackResolved => "decided by backtracking search (explored alternatives)",
        DecisionClass::Conflict => "CONFLICT — the solver could not find a valid ownership tier",
        DecisionClass::BudgetCapped => "BUDGET EXCEEDED — analysis was cut short",
        DecisionClass::BoundaryStopped => "BOUNDARY — depends on external crate, cannot infer",
    };
    let confidence_desc = match decision.confidence {
        Confidence::High => "high confidence",
        Confidence::Medium => "medium confidence",
        Confidence::Low => "low confidence — review recommended",
        Confidence::None => "no confidence — manual review required",
    };

    format!(
        "`{}` → {} ({}). {} [{}]",
        decision.binding_name, tier_desc, decision.tier_debug(), class_desc, confidence_desc
    )
}

fn explain_tier(tier: OwnershipTier) -> &'static str {
    match tier {
        OwnershipTier::PlainOwned => "plain ownership (no wrapping needed)",
        OwnershipTier::BoxOwned => "heap-allocated via Box",
        OwnershipTier::RcShared => "reference-counted sharing via Rc",
        OwnershipTier::ArcShared => "thread-safe sharing via Arc",
        OwnershipTier::RcMutShared => "mutable sharing via Rc<RefCell>",
        OwnershipTier::ArcMutShared => "thread-safe mutable sharing via Arc<Mutex>",
        OwnershipTier::Scoped => "scoped lifetime (borrow-based)",
        OwnershipTier::Undecided => "undecided (needs manual review)",
    }
}

/// Trait to get debug representation of tier on ClassifiedDecision.
trait TierDebug {
    fn tier_debug(&self) -> String;
}

impl TierDebug for ClassifiedDecision {
    fn tier_debug(&self) -> String {
        format!("{:?}", self.tier)
    }
}

/// Generate a batch of explanations.
pub fn explain_all(decisions: &[ClassifiedDecision]) -> Vec<String> {
    decisions.iter().map(explain_decision).collect()
}

/// Format K0083/K0084/K0085 messages for wiring.
pub fn format_k_code_message(code: &str, binding: &str, tier: OwnershipTier) -> String {
    match code {
        "K0083" => format!(
            "[K0083] `{}` ownership set to {:?} — low confidence, review recommended",
            binding, tier
        ),
        "K0084" => format!(
            "[K0084] `{}` ownership set to {:?} — backtracking required",
            binding, tier
        ),
        "K0085" => format!(
            "[K0085] `{}` ownership remains {:?} — manual intervention needed",
            binding, tier
        ),
        _ => format!("[{}] `{}` → {:?}", code, binding, tier),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_greedy_decision() {
        let d = ClassifiedDecision {
            node_id: kobo_ir::KirNodeId(1),
            binding_name: "data".into(),
            tier: OwnershipTier::PlainOwned,
            class: DecisionClass::Greedy,
            confidence: Confidence::High,
            explanation: String::new(),
        };
        let text = explain_decision(&d);
        assert!(text.contains("data"));
        assert!(text.contains("greedy pass"));
        assert!(text.contains("high confidence"));
    }

    #[test]
    fn k0083_message_format() {
        let msg = format_k_code_message("K0083", "counter", OwnershipTier::RcShared);
        assert!(msg.contains("K0083"));
        assert!(msg.contains("counter"));
        assert!(msg.contains("RcShared"));
    }

    #[test]
    fn batch_explain_preserves_order() {
        let decisions = vec![
            ClassifiedDecision {
                node_id: kobo_ir::KirNodeId(1),
                binding_name: "a".into(),
                tier: OwnershipTier::PlainOwned,
                class: DecisionClass::Greedy,
                confidence: Confidence::High,
                explanation: String::new(),
            },
            ClassifiedDecision {
                node_id: kobo_ir::KirNodeId(2),
                binding_name: "b".into(),
                tier: OwnershipTier::ArcShared,
                class: DecisionClass::Conflict,
                confidence: Confidence::None,
                explanation: String::new(),
            },
        ];
        let texts = explain_all(&decisions);
        assert_eq!(texts.len(), 2);
        assert!(texts[0].contains("a"));
        assert!(texts[1].contains("b"));
    }
}
