//! Stage: Provenance comments for generated Rust code.
//!
//! Generates `// kobo:` comments that can be injected into codegen output
//! to trace each ownership decision back to its source.

use crate::decision_class::{ClassifiedDecision, DecisionClass};

/// A provenance comment to inject above a binding declaration.
#[derive(Clone, Debug)]
pub struct ProvenanceComment {
    pub line_prefix: String,
    pub text: String,
}

/// Generate a provenance comment for a decision.
pub fn provenance_comment(decision: &ClassifiedDecision) -> ProvenanceComment {
    let source = match decision.class {
        DecisionClass::Greedy => "greedy",
        DecisionClass::LatticeUnique => "solver:lattice",
        DecisionClass::BacktrackResolved => "solver:backtrack",
        DecisionClass::Conflict => "conflict",
        DecisionClass::BudgetCapped => "budget-exceeded",
        DecisionClass::BoundaryStopped => "boundary",
    };
    let text = format!(
        "// kobo: {} → {:?} [{}]",
        decision.binding_name, decision.tier, source
    );
    ProvenanceComment {
        line_prefix: "    ".to_owned(),
        text,
    }
}

/// Generate provenance comments for all decisions.
pub fn provenance_comments(decisions: &[ClassifiedDecision]) -> Vec<ProvenanceComment> {
    decisions.iter().map(provenance_comment).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_class::Confidence;
    use kobo_ir::OwnershipTier;

    #[test]
    fn comment_contains_binding_name() {
        let d = ClassifiedDecision {
            node_id: kobo_ir::KirNodeId(1),
            binding_name: "counter".into(),
            tier: OwnershipTier::RcShared,
            class: DecisionClass::Greedy,
            confidence: Confidence::High,
            explanation: String::new(),
        };
        let c = provenance_comment(&d);
        assert!(c.text.contains("counter"));
        assert!(c.text.contains("greedy"));
        assert!(c.text.starts_with("// kobo:"));
    }
}
