//! Stage: Decision profile — aggregate statistics over decisions.
//!
//! Summarizes solver decisions into a profile that helps users
//! gauge migration readiness.

use crate::decision_class::{ClassifiedDecision, Confidence, DecisionClass};

/// Aggregate profile of all decisions.
#[derive(Clone, Debug, Default)]
pub struct DecisionProfile {
    pub total: usize,
    pub greedy_count: usize,
    pub solver_count: usize,
    pub conflict_count: usize,
    pub budget_capped_count: usize,
    pub boundary_stopped_count: usize,
    pub high_confidence: usize,
    pub medium_confidence: usize,
    pub low_confidence: usize,
    pub no_confidence: usize,
}

impl DecisionProfile {
    /// Compute profile from classified decisions.
    pub fn from_decisions(decisions: &[ClassifiedDecision]) -> Self {
        let mut profile = Self {
            total: decisions.len(),
            ..Default::default()
        };

        for d in decisions {
            match d.class {
                DecisionClass::Greedy => profile.greedy_count += 1,
                DecisionClass::LatticeUnique | DecisionClass::BacktrackResolved => {
                    profile.solver_count += 1
                }
                DecisionClass::Conflict => profile.conflict_count += 1,
                DecisionClass::BudgetCapped => profile.budget_capped_count += 1,
                DecisionClass::BoundaryStopped => profile.boundary_stopped_count += 1,
            }
            match d.confidence {
                Confidence::High => profile.high_confidence += 1,
                Confidence::Medium => profile.medium_confidence += 1,
                Confidence::Low => profile.low_confidence += 1,
                Confidence::None => profile.no_confidence += 1,
            }
        }

        profile
    }

    /// Migration readiness as a percentage (0.0 — 1.0).
    pub fn readiness(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        let decided = self.greedy_count + self.solver_count;
        decided as f64 / self.total as f64
    }

    /// Human-readable summary line.
    pub fn summary_line(&self) -> String {
        format!(
            "{}/{} decided ({:.0}% ready) | {} greedy, {} solver, {} conflict, {} capped",
            self.greedy_count + self.solver_count,
            self.total,
            self.readiness() * 100.0,
            self.greedy_count,
            self.solver_count,
            self.conflict_count,
            self.budget_capped_count,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_class::ClassifiedDecision;
    use kobo_ir::{KirNodeId, OwnershipTier};

    #[test]
    fn empty_profile_is_100_ready() {
        let profile = DecisionProfile::from_decisions(&[]);
        assert_eq!(profile.readiness(), 1.0);
    }

    #[test]
    fn all_greedy_is_100_ready() {
        let decisions = vec![ClassifiedDecision {
            node_id: KirNodeId(1),
            binding_name: "x".into(),
            tier: OwnershipTier::PlainOwned,
            class: DecisionClass::Greedy,
            confidence: Confidence::High,
            explanation: String::new(),
        }];
        let profile = DecisionProfile::from_decisions(&decisions);
        assert_eq!(profile.readiness(), 1.0);
        assert_eq!(profile.greedy_count, 1);
    }

    #[test]
    fn conflict_reduces_readiness() {
        let decisions = vec![
            ClassifiedDecision {
                node_id: KirNodeId(1),
                binding_name: "x".into(),
                tier: OwnershipTier::PlainOwned,
                class: DecisionClass::Greedy,
                confidence: Confidence::High,
                explanation: String::new(),
            },
            ClassifiedDecision {
                node_id: KirNodeId(2),
                binding_name: "y".into(),
                tier: OwnershipTier::Undecided,
                class: DecisionClass::Conflict,
                confidence: Confidence::None,
                explanation: String::new(),
            },
        ];
        let profile = DecisionProfile::from_decisions(&decisions);
        assert_eq!(profile.readiness(), 0.5);
    }
}
