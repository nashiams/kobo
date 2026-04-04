use kobo_ir::debt::{DebtComplexityTier, WarnEarlyFact, WarnEarlyPattern};
use kobo_ir::{Kir, KirNodeId};

/// Assign a migration complexity estimate to one `RcMutShared` KIR site.
///
/// Rules (highest tier wins):
/// - **Tier 3** — K0080-P4 (self-referential struct without Rc/Box indirection)
///   or an unsolvable escape kind present in the site's binding facts.
/// - **Tier 2** — K0080-P1, P2, or P3 associated; OR the site is mutated from
///   2 or more distinct function boundaries.
/// - **Tier 1** — no K0080-P patterns, mutated from at most 1 call site.
pub fn classify_site(
    site: KirNodeId,
    kir: &Kir,
    warn_facts: &[WarnEarlyFact],
) -> DebtComplexityTier {
    // --- Tier 3: P4 or unsolvable escape ---
    for fact in warn_facts {
        if fact.node_id == site && !fact.suppressed {
            if matches!(fact.pattern, WarnEarlyPattern::SelfReferentialStruct { .. }) {
                return DebtComplexityTier::Tier3;
            }
        }
    }

    // --- Tier 2: P1, P2, or P3 patterns ---
    for fact in warn_facts {
        if fact.node_id == site && !fact.suppressed {
            match &fact.pattern {
                WarnEarlyPattern::BidirectionalRcLinks { .. }
                | WarnEarlyPattern::ParentChildBackPointer { .. }
                | WarnEarlyPattern::SharedMutableAt3PlusSites { .. } => {
                    return DebtComplexityTier::Tier2;
                }
                _ => {}
            }
        }
    }

    // --- Tier 2: 2+ mutation boundaries from transform facts ---
    let binding = kir
        .transform_facts()
        .bindings
        .iter()
        .find(|b| b.node == site);
    if let Some(b) = binding {
        if b.shared_facts.mutable_sites >= 2 {
            return DebtComplexityTier::Tier2;
        }
    }

    DebtComplexityTier::Tier1
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

    fn make_kir_with_node(id: u32, tier: OwnershipTier) -> Kir {
        let node = KirNode {
            id: KirNodeId(id),
            kind: NodeKind::Decl,
            ast_id: None,
            ownership: tier,
            resource_kind: None,
            cfg_block: None,
            span: dummy_span(),
            decl_id: None,
        };
        Kir::from_nodes(vec![node])
    }

    fn p4_fact(node_id: u32) -> WarnEarlyFact {
        WarnEarlyFact {
            node_id: KirNodeId(node_id),
            span: dummy_span(),
            pattern: WarnEarlyPattern::SelfReferentialStruct {
                struct_name: "Cyclic".to_owned(),
            },
            suppressed: false,
            known_debt_reason: None,
            struct_name: "Cyclic".to_owned(),
        }
    }

    fn p1_fact(node_id: u32) -> WarnEarlyFact {
        WarnEarlyFact {
            node_id: KirNodeId(node_id),
            span: dummy_span(),
            pattern: WarnEarlyPattern::BidirectionalRcLinks {
                struct_name: "Node".to_owned(),
                field_pairs: vec![("a".to_owned(), "b".to_owned())],
            },
            suppressed: false,
            known_debt_reason: None,
            struct_name: "Node".to_owned(),
        }
    }

    fn p3_fact(node_id: u32) -> WarnEarlyFact {
        WarnEarlyFact {
            node_id: KirNodeId(node_id),
            span: dummy_span(),
            pattern: WarnEarlyPattern::SharedMutableAt3PlusSites {
                binding_id: KirNodeId(node_id),
                site_count: 3,
            },
            suppressed: false,
            known_debt_reason: None,
            struct_name: String::new(),
        }
    }

    #[test]
    fn zero_patterns_is_tier1() {
        let kir = make_kir_with_node(1, OwnershipTier::RcMutShared);
        let tier = classify_site(KirNodeId(1), &kir, &[]);
        assert_eq!(tier, DebtComplexityTier::Tier1);
    }

    #[test]
    fn p1_pattern_is_tier2() {
        let kir = make_kir_with_node(1, OwnershipTier::RcMutShared);
        let facts = [p1_fact(1)];
        let tier = classify_site(KirNodeId(1), &kir, &facts);
        assert_eq!(tier, DebtComplexityTier::Tier2);
    }

    #[test]
    fn p4_pattern_is_tier3() {
        let kir = make_kir_with_node(1, OwnershipTier::RcMutShared);
        let facts = [p4_fact(1)];
        let tier = classify_site(KirNodeId(1), &kir, &facts);
        assert_eq!(tier, DebtComplexityTier::Tier3);
    }

    #[test]
    fn p4_overrides_p1_to_tier3() {
        let kir = make_kir_with_node(1, OwnershipTier::RcMutShared);
        let facts = [p1_fact(1), p4_fact(1)];
        let tier = classify_site(KirNodeId(1), &kir, &facts);
        assert_eq!(tier, DebtComplexityTier::Tier3);
    }

    #[test]
    fn p1_and_p3_is_tier2() {
        let kir = make_kir_with_node(1, OwnershipTier::RcMutShared);
        let facts = [p1_fact(1), p3_fact(1)];
        let tier = classify_site(KirNodeId(1), &kir, &facts);
        assert_eq!(tier, DebtComplexityTier::Tier2);
    }

    #[test]
    fn suppressed_p4_falls_to_tier1() {
        let kir = make_kir_with_node(1, OwnershipTier::RcMutShared);
        let mut fact = p4_fact(1);
        fact.suppressed = true;
        let facts = [fact];
        let tier = classify_site(KirNodeId(1), &kir, &facts);
        assert_eq!(tier, DebtComplexityTier::Tier1);
    }
}
