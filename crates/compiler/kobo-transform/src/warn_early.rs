// warn_early.rs — K0080-P structural ownership pattern detector.
//
// All four v0.4 patterns are detected here from frozen KIR data:
//
//   P1: BidirectionalRcLinks       — Rc cycle between two struct types
//   P2: ParentChildBackPointer     — Vec<Rc<RefCell<Self>>> + Option<Rc<RefCell<Self>>>
//   P3: SharedMutableAt3PlusSites  — binding mutated from ≥3 distinct call sites
//   P4: SelfReferentialStruct      — direct (non-indirected) self reference
//
// Priority: P4 is emitted first, P2 supersedes P1 when both match the same struct.
// P3 is binding-level, the others are struct-level.
//
// Trap 7: must NOT emit KDiagnostic here — only produce WarnEarlyFact values.
// Trap 13: P3 site_count comes from SharedBindingFacts.mutable_sites (function
//           boundaries), not raw mutation event counts.
// Contract C04: types consumed here (KirStructDef, WarnEarlyFact) live in kobo-ir.

use kobo_ir::{
    FieldTypeShape, Kir, KirNodeId, WarnEarlyFact, WarnEarlyPattern,
};

/// Detect all K0080-P structural patterns in `kir` and return one `WarnEarlyFact`
/// per detected pattern instance.
///
/// This function is called once per source file, after `kir.transform_facts()`,
/// `kir.struct_defs()`, and `kir.tier_decisions()` are all set.
pub fn detect_warn_early(kir: &Kir) -> Vec<WarnEarlyFact> {
    let mut facts: Vec<WarnEarlyFact> = Vec::new();

    detect_p4_self_referential(kir, &mut facts);
    detect_p2_parent_child(kir, &mut facts);
    detect_p1_bidirectional(kir, &mut facts);
    detect_p3_shared_mutable(kir, &mut facts);

    facts
}

// ---------------------------------------------------------------------------
// P4: SelfReferentialStruct
// ---------------------------------------------------------------------------

/// K0080-P4: struct with a direct (non-indirected) self-referential field.
///
/// `struct Chain { next: Chain }` — infinite size without Box/Rc indirection.
/// `struct Node { child: Rc<RefCell<Node>> }` does NOT trigger P4.
fn detect_p4_self_referential(kir: &Kir, out: &mut Vec<WarnEarlyFact>) {
    for def in kir.struct_defs() {
        let is_self_ref = def.fields.iter().any(|f| {
            matches!(&f.shape, FieldTypeShape::DirectNamed(name) if name == &def.name)
        });
        if !is_self_ref {
            continue;
        }
        out.push(WarnEarlyFact {
            node_id: KirNodeId(0),
            span: def.span,
            pattern: WarnEarlyPattern::SelfReferentialStruct {
                struct_name: def.name.clone(),
            },
            suppressed: def.known_debt_reason.is_some(),
            known_debt_reason: def.known_debt_reason.clone(),
            struct_name: def.name.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// P2: ParentChildBackPointer
// ---------------------------------------------------------------------------

/// K0080-P2: struct with both `Vec<Rc<RefCell<Self>>>` (children) and
/// `Option<Rc<RefCell<Self>>>` (parent) fields, where "Self" means the same
/// struct name as the container.
///
/// P2 supersedes P1 when both would fire on the same struct — P2 is more
/// specific and actionable.
fn detect_p2_parent_child(kir: &Kir, out: &mut Vec<WarnEarlyFact>) {
    for def in kir.struct_defs() {
        let children_field = def.fields.iter().find(|f| {
            matches!(&f.shape, FieldTypeShape::VecRcRefCellOf(name)
                if name == &def.name || name == "Self")
        });
        let parent_field = def.fields.iter().find(|f| {
            matches!(&f.shape, FieldTypeShape::OptionRcRefCellOf(name)
                if name == &def.name || name == "Self")
        });
        let (cf, pf) = match (children_field, parent_field) {
            (Some(cf), Some(pf)) => (cf, pf),
            _ => continue,
        };
        out.push(WarnEarlyFact {
            node_id: KirNodeId(0),
            span: def.span,
            pattern: WarnEarlyPattern::ParentChildBackPointer {
                struct_name: def.name.clone(),
                children_field: cf.name.clone(),
                parent_field: pf.name.clone(),
            },
            suppressed: def.known_debt_reason.is_some(),
            known_debt_reason: def.known_debt_reason.clone(),
            struct_name: def.name.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// P1: BidirectionalRcLinks
// ---------------------------------------------------------------------------

/// K0080-P1: two structs each holding `Rc<RefCell<Other>>` to the other,
/// forming an Rc reference cycle that will leak memory.
///
/// Within a single struct, two `Rc<RefCell<Self>>` fields also count (each
/// field can form a back-link to a clone of the same object).
///
/// Structs that already have P2 detected are skipped — P2 supersedes P1.
fn detect_p1_bidirectional(kir: &Kir, out: &mut Vec<WarnEarlyFact>) {
    let struct_defs = kir.struct_defs();

    // Collect P2 struct names into an owned set (avoids holding &out borrow).
    let p2_structs: std::collections::HashSet<String> = out
        .iter()
        .filter_map(|f| {
            if let WarnEarlyPattern::ParentChildBackPointer { struct_name, .. } = &f.pattern {
                Some(struct_name.clone())
            } else {
                None
            }
        })
        .collect();

    for def in struct_defs {
        if p2_structs.contains(&def.name) {
            continue;
        }

        // Collect all Rc-family fields for this struct.
        let rc_fields: Vec<(&str, &str)> = def
            .fields
            .iter()
            .filter_map(|f| match &f.shape {
                FieldTypeShape::RcRefCellOf(name)
                | FieldTypeShape::OptionRcRefCellOf(name)
                | FieldTypeShape::VecRcRefCellOf(name) => Some((f.name.as_str(), name.as_str())),
                _ => None,
            })
            .collect();

        if rc_fields.is_empty() {
            continue;
        }

        let mut field_pairs: Vec<(String, String)> = Vec::new();

        // --- Case 1: within-struct self-loops (2+ fields pointing to Self) ---
        for (i, (field_a, target_a)) in rc_fields.iter().enumerate() {
            for (field_b, target_b) in rc_fields.iter().skip(i + 1) {
                let a_is_self =
                    *target_a == def.name.as_str() || *target_a == "Self";
                let b_is_self =
                    *target_b == def.name.as_str() || *target_b == "Self";
                if a_is_self && b_is_self {
                    field_pairs.push(((*field_a).to_owned(), (*field_b).to_owned()));
                }
            }
        }

        // --- Case 2: cross-struct back-links ---
        // For each Rc field in `def` pointing to type B, check whether B has
        // any Rc field pointing back to `def`. Emit a pair (field_in_def, back_field_in_B).
        for (field_name, target_name) in &rc_fields {
            // Skip pure self-references — those are handled above.
            if *target_name == def.name.as_str() || *target_name == "Self" {
                continue;
            }
            let canonical_target = *target_name;
            if let Some(other) = struct_defs.iter().find(|d| d.name == canonical_target) {
                for other_field in &other.fields {
                    let back_target = match &other_field.shape {
                        FieldTypeShape::RcRefCellOf(n)
                        | FieldTypeShape::OptionRcRefCellOf(n)
                        | FieldTypeShape::VecRcRefCellOf(n) => n.as_str(),
                        _ => continue,
                    };
                    if back_target == def.name.as_str() || back_target == "Self" {
                        field_pairs.push((
                            (*field_name).to_owned(),
                            other_field.name.clone(),
                        ));
                    }
                }
            }
        }

        // Deduplicate (within-struct and cross-struct may find same pair twice).
        field_pairs.sort_unstable();
        field_pairs.dedup();

        if !field_pairs.is_empty() {
            out.push(WarnEarlyFact {
                node_id: KirNodeId(0),
                span: def.span,
                pattern: WarnEarlyPattern::BidirectionalRcLinks {
                    struct_name: def.name.clone(),
                    field_pairs,
                },
                suppressed: def.known_debt_reason.is_some(),
                known_debt_reason: def.known_debt_reason.clone(),
                struct_name: def.name.clone(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// P3: SharedMutableAt3PlusSites
// ---------------------------------------------------------------------------

/// K0080-P3: a binding that is mutably accessed from ≥3 distinct function
/// boundaries (call sites).
///
/// `mutable_sites` in `SharedBindingFacts` counts distinct caller function
/// identities, not raw mutation events (Trap 13).
fn detect_p3_shared_mutable(kir: &Kir, out: &mut Vec<WarnEarlyFact>) {
    for binding in kir.transform_facts().bindings.iter() {
        if binding.shared_facts.mutable_sites < 3 {
            continue;
        }
        out.push(WarnEarlyFact {
            node_id: binding.node,
            span: binding.span,
            pattern: WarnEarlyPattern::SharedMutableAt3PlusSites {
                binding_id: binding.node,
                site_count: binding.shared_facts.mutable_sites,
            },
            // Binding-level suppression via #[kobo::known_debt] is planned for
            // v0.5; for now, binding-level facts are never suppressed.
            suppressed: false,
            known_debt_reason: None,
            struct_name: binding.binding_name.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{FieldTypeShape, KirStructDef, KirStructFieldDef, KoboSpan};

    fn dummy_span() -> KoboSpan {
        use kobo_ir::FileId;
        KoboSpan::new(0, 1, FileId(0))
    }

    fn make_kir_with_structs(defs: Vec<KirStructDef>) -> Kir {
        let mut kir = Kir::default();
        kir.set_struct_defs(defs);
        kir
    }

    #[test]
    fn p4_detects_direct_self_reference() {
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Chain".to_owned(),
            span: dummy_span(),
            fields: vec![KirStructFieldDef {
                name: "next".to_owned(),
                shape: FieldTypeShape::DirectNamed("Chain".to_owned()),
            }],
            known_debt_reason: None,
            known_debt_span: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert_eq!(facts.len(), 1);
        assert!(matches!(
            &facts[0].pattern,
            WarnEarlyPattern::SelfReferentialStruct { struct_name } if struct_name == "Chain"
        ));
    }

    #[test]
    fn p4_rc_refcell_self_does_not_trigger_p4() {
        // Rc<RefCell<Self>> is normal and should not fire P4.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Node".to_owned(),
            span: dummy_span(),
            fields: vec![KirStructFieldDef {
                name: "child".to_owned(),
                shape: FieldTypeShape::RcRefCellOf("Node".to_owned()),
            }],
            known_debt_reason: None,
            known_debt_span: None,
        }]);
        let facts = detect_warn_early(&kir);
        // P4 should not fire; P1 may fire (self-loop Rc field)
        assert!(!facts.iter().any(|f| matches!(
            &f.pattern,
            WarnEarlyPattern::SelfReferentialStruct { .. }
        )));
    }

    #[test]
    fn p2_detects_parent_child_tree() {
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "TreeNode".to_owned(),
            span: dummy_span(),
            fields: vec![
                KirStructFieldDef {
                    name: "children".to_owned(),
                    shape: FieldTypeShape::VecRcRefCellOf("TreeNode".to_owned()),
                },
                KirStructFieldDef {
                    name: "parent".to_owned(),
                    shape: FieldTypeShape::OptionRcRefCellOf("TreeNode".to_owned()),
                },
            ],
            known_debt_reason: None,
            known_debt_span: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert!(facts.iter().any(|f| matches!(
            &f.pattern,
            WarnEarlyPattern::ParentChildBackPointer {
                struct_name,
                children_field,
                parent_field,
            } if struct_name == "TreeNode"
                && children_field == "children"
                && parent_field == "parent"
        )));
        // P1 must NOT also fire for the same struct (P2 supersedes P1).
        assert!(!facts.iter().any(|f| matches!(
            &f.pattern,
            WarnEarlyPattern::BidirectionalRcLinks { struct_name, .. }
                if struct_name == "TreeNode"
        )));
    }

    #[test]
    fn p1_detects_bidirectional_rc_links() {
        let kir = make_kir_with_structs(vec![
            KirStructDef {
                name: "A".to_owned(),
                span: dummy_span(),
                fields: vec![KirStructFieldDef {
                    name: "b_ref".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("B".to_owned()),
                }],
                known_debt_reason: None,
                known_debt_span: None,
            },
            KirStructDef {
                name: "B".to_owned(),
                span: dummy_span(),
                fields: vec![KirStructFieldDef {
                    name: "a_ref".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("A".to_owned()),
                }],
                known_debt_reason: None,
                known_debt_span: None,
            },
        ]);
        let facts = detect_warn_early(&kir);
        assert!(facts.iter().any(|f| matches!(
            &f.pattern,
            WarnEarlyPattern::BidirectionalRcLinks { .. }
        )));
    }

    #[test]
    fn known_debt_sets_suppressed() {
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Chain".to_owned(),
            span: dummy_span(),
            fields: vec![KirStructFieldDef {
                name: "next".to_owned(),
                shape: FieldTypeShape::DirectNamed("Chain".to_owned()),
            }],
            known_debt_reason: Some("intentional for v0.5 refactor".to_owned()),
            known_debt_span: Some(dummy_span()),
        }]);
        let facts = detect_warn_early(&kir);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].suppressed);
        assert_eq!(
            facts[0].known_debt_reason.as_deref(),
            Some("intentional for v0.5 refactor")
        );
    }
}
