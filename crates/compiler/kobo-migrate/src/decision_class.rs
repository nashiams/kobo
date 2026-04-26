//! Phase 11: Decision classification for human review.
//!
//! Classifies solver decisions into categories that help users understand
//! what the solver did and why.

use kobo_ir::{KirNodeId, OwnershipTier};

/// Classification of a solver decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionClass {
    /// Greedy pass decided — no solver involvement.
    Greedy,
    /// Lattice solver decided uniquely.
    LatticeUnique,
    /// Backtracking search found a solution.
    BacktrackResolved,
    /// Conflict: solver could not find a valid assignment.
    Conflict,
    /// Budget exceeded before a decision was reached.
    BudgetCapped,
    /// Boundary stop — cross-crate dependency prevented solving.
    BoundaryStopped,
}

/// A classified decision for one binding.
#[derive(Clone, Debug)]
pub struct ClassifiedDecision {
    pub node_id: KirNodeId,
    pub binding_name: String,
    pub tier: OwnershipTier,
    pub class: DecisionClass,
    pub confidence: Confidence,
    pub explanation: String,
}

/// Confidence level for a decision.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Confidence {
    /// Very high confidence (greedy, obvious choice).
    High,
    /// Medium confidence (lattice unique, no ambiguity).
    Medium,
    /// Low confidence (backtracking, multiple alternatives existed).
    Low,
    /// No confidence (conflict or budget exceeded).
    None,
}

/// Classify a set of decisions.
pub fn classify_decisions(
    greedy_resolved: &[(KirNodeId, OwnershipTier, String)],
    solver_resolved: &[(KirNodeId, OwnershipTier, String)],
    conflicts: &[KirNodeId],
    budget_exceeded: &[KirNodeId],
) -> Vec<ClassifiedDecision> {
    let mut result = Vec::new();

    for (node_id, tier, name) in greedy_resolved {
        result.push(ClassifiedDecision {
            node_id: *node_id,
            binding_name: name.clone(),
            tier: *tier,
            class: DecisionClass::Greedy,
            confidence: Confidence::High,
            explanation: format!("Greedy pass decided {:?}", tier),
        });
    }

    for (node_id, tier, name) in solver_resolved {
        result.push(ClassifiedDecision {
            node_id: *node_id,
            binding_name: name.clone(),
            tier: *tier,
            class: DecisionClass::LatticeUnique,
            confidence: Confidence::Medium,
            explanation: format!("Solver decided {:?}", tier),
        });
    }

    for &node_id in conflicts {
        result.push(ClassifiedDecision {
            node_id,
            binding_name: String::new(),
            tier: OwnershipTier::Undecided,
            class: DecisionClass::Conflict,
            confidence: Confidence::None,
            explanation: "Solver conflict — no valid assignment".to_owned(),
        });
    }

    for &node_id in budget_exceeded {
        result.push(ClassifiedDecision {
            node_id,
            binding_name: String::new(),
            tier: OwnershipTier::Undecided,
            class: DecisionClass::BudgetCapped,
            confidence: Confidence::None,
            explanation: "Budget exceeded before decision".to_owned(),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greedy_classified_as_high_confidence() {
        let decisions = classify_decisions(
            &[(KirNodeId(1), OwnershipTier::PlainOwned, "x".into())],
            &[],
            &[],
            &[],
        );
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].class, DecisionClass::Greedy);
        assert_eq!(decisions[0].confidence, Confidence::High);
    }

    #[test]
    fn conflicts_classified_as_no_confidence() {
        let decisions = classify_decisions(&[], &[], &[KirNodeId(5)], &[]);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].confidence, Confidence::None);
    }
}
