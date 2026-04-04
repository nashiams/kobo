use kobo_ir::debt::{
    AcknowledgedDebtRecord, ComplexityBreakdown, DebtComplexityTier, DebtReport, DebtSiteRecord,
    WrapperInventory,
};
use kobo_ir::{Kir, OwnershipTier};

use crate::complexity::classify_site;

/// Build the complete ownership debt report from a frozen KIR.
///
/// This function is read-only — it does not modify the KIR or write to the
/// filesystem. All inventory counts come from KIR `OwnershipTier` nodes;
/// no text search over generated `.rs` files is performed (Contract C05).
///
/// `file_count` and `line_count` are metadata supplied by the caller (driver
/// or CLI) from the compile session context.
pub fn build_debt_report(kir: &Kir, file_count: usize, line_count: usize) -> DebtReport {
    let mut report = DebtReport::new();
    report.file_count = file_count;
    report.line_count = line_count;

    // --- Wrapper inventory: walk all declaration nodes and count by tier ---
    let mut inventory = WrapperInventory::default();
    let warn_facts = kir.warn_early_facts();

    for node in kir.iter_decl_nodes() {
        match node.ownership {
            OwnershipTier::PlainOwned => inventory.plain_owned += 1,
            OwnershipTier::BoxOwned => inventory.box_owned += 1,
            OwnershipTier::RcShared => inventory.rc_shared += 1,
            OwnershipTier::ArcShared => inventory.arc_shared += 1,
            OwnershipTier::RcMutShared => inventory.rc_mut_shared += 1,
            _ => {}
        }
    }
    report.inventory = inventory;

    // --- Per-site records for each RcMutShared node ---
    let mut complexity = ComplexityBreakdown::default();

    for node in kir.iter_decl_nodes() {
        if node.ownership != OwnershipTier::RcMutShared {
            continue;
        }

        let tier = classify_site(node.id, kir, warn_facts);

        // Collect warn_early patterns associated with this KIR node.
        let node_patterns: Vec<_> = warn_facts
            .iter()
            .filter(|f| f.node_id == node.id && !f.suppressed)
            .map(|f| f.pattern.clone())
            .collect();

        let suppressed = warn_facts
            .iter()
            .any(|f| f.node_id == node.id && f.suppressed);

        // Look up the binding name from transform facts.
        let binding_name = kir
            .transform_facts()
            .bindings
            .iter()
            .find(|b| b.node == node.id)
            .map(|b| b.binding_name.clone())
            .unwrap_or_else(|| format!("_{}", node.id.0));

        let record = DebtSiteRecord {
            node_id: node.id,
            span: node.span,
            tier: node.ownership,
            complexity_estimate: tier.clone(),
            warn_early: node_patterns,
            suppressed,
            binding_name,
        };
        report.sites.push(record);

        match tier {
            DebtComplexityTier::Tier1 => complexity.tier1 += 1,
            DebtComplexityTier::Tier2 => complexity.tier2 += 1,
            DebtComplexityTier::Tier3 => complexity.tier3 += 1,
        }
    }
    report.complexity = complexity;

    // --- Warn-early and acknowledged facts ---
    for fact in kir.warn_early_facts() {
        if fact.suppressed {
            report.acknowledged.push(AcknowledgedDebtRecord {
                span: fact.span,
                reason: fact
                    .known_debt_reason
                    .clone()
                    .unwrap_or_default(),
                pattern: fact.pattern.clone(),
                struct_name: fact.struct_name.clone(),
            });
        } else {
            report.warn_early.push(fact.clone());
        }
    }

    report
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::debt::{WarnEarlyFact, WarnEarlyPattern};
    use kobo_ir::{FileId, KirNodeId, KoboSpan};
    use kobo_ir::{Kir, KirNode, NodeKind, OwnershipTier};

    fn dummy_span() -> KoboSpan {
        KoboSpan::new(0, 1, FileId(0))
    }

    fn make_node(id: u32, tier: OwnershipTier) -> KirNode {
        KirNode {
            id: KirNodeId(id),
            kind: NodeKind::Decl,
            ast_id: None,
            ownership: tier,
            resource_kind: None,
            cfg_block: None,
            span: dummy_span(),
            decl_id: None,
        }
    }

    #[test]
    fn inventory_counts_match_exactly() {
        let nodes = vec![
            make_node(1, OwnershipTier::PlainOwned),
            make_node(2, OwnershipTier::PlainOwned),
            make_node(3, OwnershipTier::PlainOwned),
            make_node(4, OwnershipTier::RcShared),
            make_node(5, OwnershipTier::RcShared),
            make_node(6, OwnershipTier::RcMutShared),
            make_node(7, OwnershipTier::RcMutShared),
            make_node(8, OwnershipTier::RcMutShared),
            make_node(9, OwnershipTier::RcMutShared),
        ];
        let kir = Kir::from_nodes(nodes);
        let report = build_debt_report(&kir, 1, 100);
        assert_eq!(report.inventory.plain_owned, 3);
        assert_eq!(report.inventory.rc_shared, 2);
        assert_eq!(report.inventory.rc_mut_shared, 4);
    }

    #[test]
    fn complexity_breakdown_invariant_holds() {
        let nodes = vec![
            make_node(1, OwnershipTier::RcMutShared),
            make_node(2, OwnershipTier::RcMutShared),
            make_node(3, OwnershipTier::RcMutShared),
        ];
        let kir = Kir::from_nodes(nodes);
        let report = build_debt_report(&kir, 1, 50);
        let sum = report.complexity.tier1 + report.complexity.tier2 + report.complexity.tier3;
        assert_eq!(
            sum,
            report.inventory.rc_mut_shared,
            "tier1 + tier2 + tier3 must equal rc_mut_shared"
        );
    }

    #[test]
    fn active_warn_fact_goes_to_warn_early() {
        let nodes = vec![make_node(1, OwnershipTier::RcMutShared)];
        let mut kir = Kir::from_nodes(nodes);
        kir.set_warn_early_facts(vec![WarnEarlyFact {
            node_id: KirNodeId(1),
            span: dummy_span(),
            pattern: WarnEarlyPattern::SelfReferentialStruct {
                struct_name: "Chain".to_owned(),
            },
            suppressed: false,
            known_debt_reason: None,
            struct_name: "Chain".to_owned(),
        }]);
        let report = build_debt_report(&kir, 1, 10);
        assert_eq!(report.warn_early.len(), 1);
        assert_eq!(report.acknowledged.len(), 0);
    }

    #[test]
    fn suppressed_warn_fact_goes_to_acknowledged() {
        let nodes = vec![make_node(1, OwnershipTier::RcMutShared)];
        let mut kir = Kir::from_nodes(nodes);
        kir.set_warn_early_facts(vec![WarnEarlyFact {
            node_id: KirNodeId(1),
            span: dummy_span(),
            pattern: WarnEarlyPattern::BidirectionalRcLinks {
                struct_name: "Node".to_owned(),
                field_pairs: vec![("a".to_owned(), "b".to_owned())],
            },
            suppressed: true,
            known_debt_reason: Some("will use arena".to_owned()),
            struct_name: "Node".to_owned(),
        }]);
        let report = build_debt_report(&kir, 1, 10);
        assert_eq!(report.warn_early.len(), 0);
        assert_eq!(report.acknowledged.len(), 1);
        assert_eq!(report.acknowledged[0].reason, "will use arena");
    }

    #[test]
    fn schema_version_is_one() {
        let kir = Kir::from_nodes(vec![]);
        let report = build_debt_report(&kir, 0, 0);
        assert_eq!(report.schema_version, 1);
    }
}
