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

use kobo_ir::{FieldTypeShape, Kir, KirNodeId, WarnEarlyFact, WarnEarlyPattern};

/// Detect K0080-P1 through K0080-P4 structural ownership patterns.
///
/// # Dependencies
///
/// This function reads `kir.transform_facts()` and `kir.iter_tier_decisions()`
/// internally. The caller must ensure `Kir::set_transform_facts()` and
/// `Kir::set_tier_decisions()` have been called before invoking this function.
/// See `transform.rs::build_kir()` for the correct call sequence.
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
        let is_self_ref = def
            .fields
            .iter()
            .any(|f| matches!(&f.shape, FieldTypeShape::DirectNamed(name) if name == &def.name));
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
                let a_is_self = *target_a == def.name.as_str() || *target_a == "Self";
                let b_is_self = *target_b == def.name.as_str() || *target_b == "Self";
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
                        field_pairs.push(((*field_name).to_owned(), other_field.name.clone()));
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
            known_debt_parse_error: None,
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
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        // P4 should not fire; P1 may fire (self-loop Rc field)
        assert!(!facts
            .iter()
            .any(|f| matches!(&f.pattern, WarnEarlyPattern::SelfReferentialStruct { .. })));
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
            known_debt_parse_error: None,
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
                known_debt_parse_error: None,
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
                known_debt_parse_error: None,
            },
        ]);
        let facts = detect_warn_early(&kir);
        assert!(facts
            .iter()
            .any(|f| matches!(&f.pattern, WarnEarlyPattern::BidirectionalRcLinks { .. })));
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
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].suppressed);
        assert_eq!(
            facts[0].known_debt_reason.as_deref(),
            Some("intentional for v0.5 refactor")
        );
    }

    // --- P1 additional tests ---

    #[test]
    fn p1_within_struct_self_loop() {
        // Two Rc<RefCell<Self>> fields in the same struct form a self-loop pair.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Graph".to_owned(),
            span: dummy_span(),
            fields: vec![
                KirStructFieldDef {
                    name: "left".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Graph".to_owned()),
                },
                KirStructFieldDef {
                    name: "right".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Graph".to_owned()),
                },
            ],
            known_debt_reason: None,
            known_debt_span: None,
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        let p1 = facts
            .iter()
            .find(|f| matches!(&f.pattern, WarnEarlyPattern::BidirectionalRcLinks { .. }));
        assert!(p1.is_some(), "P1 should fire for within-struct self-loop");
        if let WarnEarlyPattern::BidirectionalRcLinks { field_pairs, .. } = &p1.unwrap().pattern {
            assert_eq!(field_pairs.len(), 1);
            assert_eq!(field_pairs[0], ("left".to_owned(), "right".to_owned()));
        }
    }

    #[test]
    fn p1_multi_pair_dedup() {
        // Three Rc<RefCell<Self>> fields produce 3 pairs, deduplicated.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Mesh".to_owned(),
            span: dummy_span(),
            fields: vec![
                KirStructFieldDef {
                    name: "a".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Mesh".to_owned()),
                },
                KirStructFieldDef {
                    name: "b".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Mesh".to_owned()),
                },
                KirStructFieldDef {
                    name: "c".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Mesh".to_owned()),
                },
            ],
            known_debt_reason: None,
            known_debt_span: None,
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        let p1 = facts
            .iter()
            .find(|f| matches!(&f.pattern, WarnEarlyPattern::BidirectionalRcLinks { .. }));
        assert!(p1.is_some());
        if let WarnEarlyPattern::BidirectionalRcLinks { field_pairs, .. } = &p1.unwrap().pattern {
            assert_eq!(field_pairs.len(), 3, "3 self-loop pairs from 3 fields");
        }
    }

    #[test]
    fn p1_option_rc_refcell_pair_detected() {
        // Option<Rc<RefCell<T>>> fields also count for P1 detection.
        let kir = make_kir_with_structs(vec![
            KirStructDef {
                name: "X".to_owned(),
                span: dummy_span(),
                fields: vec![KirStructFieldDef {
                    name: "y_ref".to_owned(),
                    shape: FieldTypeShape::OptionRcRefCellOf("Y".to_owned()),
                }],
                known_debt_reason: None,
                known_debt_span: None,
                known_debt_parse_error: None,
            },
            KirStructDef {
                name: "Y".to_owned(),
                span: dummy_span(),
                fields: vec![KirStructFieldDef {
                    name: "x_ref".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("X".to_owned()),
                }],
                known_debt_reason: None,
                known_debt_span: None,
                known_debt_parse_error: None,
            },
        ]);
        let facts = detect_warn_early(&kir);
        assert!(facts
            .iter()
            .any(|f| matches!(&f.pattern, WarnEarlyPattern::BidirectionalRcLinks { .. })));
    }

    // --- P2 additional tests ---

    #[test]
    fn p2_children_only_no_parent() {
        // Vec<Rc<RefCell<Self>>> without Option<Rc<RefCell<Self>>> → no P2.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "List".to_owned(),
            span: dummy_span(),
            fields: vec![KirStructFieldDef {
                name: "items".to_owned(),
                shape: FieldTypeShape::VecRcRefCellOf("List".to_owned()),
            }],
            known_debt_reason: None,
            known_debt_span: None,
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert!(!facts
            .iter()
            .any(|f| matches!(&f.pattern, WarnEarlyPattern::ParentChildBackPointer { .. })));
    }

    #[test]
    fn p2_parent_only_no_children() {
        // Option<Rc<RefCell<Self>>> without Vec<Rc<RefCell<Self>>> → no P2.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Leaf".to_owned(),
            span: dummy_span(),
            fields: vec![KirStructFieldDef {
                name: "parent".to_owned(),
                shape: FieldTypeShape::OptionRcRefCellOf("Leaf".to_owned()),
            }],
            known_debt_reason: None,
            known_debt_span: None,
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert!(!facts
            .iter()
            .any(|f| matches!(&f.pattern, WarnEarlyPattern::ParentChildBackPointer { .. })));
    }

    #[test]
    fn p2_supersedes_p1_isolation() {
        // Two structs: one with P2, one with P1. P2 struct must not also get P1.
        let kir = make_kir_with_structs(vec![
            // P2 struct: has both Vec and Option self-refs.
            KirStructDef {
                name: "Tree".to_owned(),
                span: dummy_span(),
                fields: vec![
                    KirStructFieldDef {
                        name: "children".to_owned(),
                        shape: FieldTypeShape::VecRcRefCellOf("Tree".to_owned()),
                    },
                    KirStructFieldDef {
                        name: "parent".to_owned(),
                        shape: FieldTypeShape::OptionRcRefCellOf("Tree".to_owned()),
                    },
                ],
                known_debt_reason: None,
                known_debt_span: None,
                known_debt_parse_error: None,
            },
            // P1 struct: cross-links with another type.
            KirStructDef {
                name: "Foo".to_owned(),
                span: dummy_span(),
                fields: vec![KirStructFieldDef {
                    name: "bar_ref".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Bar".to_owned()),
                }],
                known_debt_reason: None,
                known_debt_span: None,
                known_debt_parse_error: None,
            },
            KirStructDef {
                name: "Bar".to_owned(),
                span: dummy_span(),
                fields: vec![KirStructFieldDef {
                    name: "foo_ref".to_owned(),
                    shape: FieldTypeShape::RcRefCellOf("Foo".to_owned()),
                }],
                known_debt_reason: None,
                known_debt_span: None,
                known_debt_parse_error: None,
            },
        ]);
        let facts = detect_warn_early(&kir);
        // Tree should have P2 only, not P1.
        assert!(facts.iter().any(|f| matches!(&f.pattern, WarnEarlyPattern::ParentChildBackPointer { struct_name, .. } if struct_name == "Tree")));
        assert!(!facts.iter().any(|f| matches!(&f.pattern, WarnEarlyPattern::BidirectionalRcLinks { struct_name, .. } if struct_name == "Tree")));
        // Foo should have P1.
        assert!(facts.iter().any(|f| matches!(&f.pattern, WarnEarlyPattern::BidirectionalRcLinks { struct_name, .. } if struct_name == "Foo")));
    }

    // --- P3 additional tests ---

    fn make_kir_with_binding(mutable_sites: usize) -> Kir {
        use kobo_ir::{
            BindingUsage, KoboAstNodeId, SharedBindingFacts, TransformBindingFacts, TransformFacts,
        };
        let mut kir = Kir::default();
        let mut facts = TransformFacts::default();
        facts.bindings.push(TransformBindingFacts {
            node: KirNodeId(1),
            ast_id: KoboAstNodeId(1),
            binding_name: "x".to_owned(),
            span: dummy_span(),
            resource_kind: None,
            hint: None,
            hint_span: None,
            is_copy_known: false,
            is_generic: false,
            is_async: false,
            async_shared: false,
            usage: BindingUsage::new(dummy_span()),
            shared_facts: SharedBindingFacts {
                node_id: KirNodeId(1),
                mutable_sites,
                ..SharedBindingFacts::default()
            },
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
            decl_scope_depth: 0,
            ref_returning_read_spans: Vec::new(),
        });
        kir.set_transform_facts(facts);
        kir
    }

    #[test]
    fn p3_below_threshold() {
        // 2 mutable sites → no P3 (threshold is 3).
        let kir = make_kir_with_binding(2);
        let facts = detect_warn_early(&kir);
        assert!(!facts.iter().any(|f| matches!(
            &f.pattern,
            WarnEarlyPattern::SharedMutableAt3PlusSites { .. }
        )));
    }

    #[test]
    fn p3_at_threshold() {
        // Exactly 3 mutable sites → P3 detected.
        let kir = make_kir_with_binding(3);
        let facts = detect_warn_early(&kir);
        let p3 = facts.iter().find(|f| {
            matches!(
                &f.pattern,
                WarnEarlyPattern::SharedMutableAt3PlusSites { .. }
            )
        });
        assert!(p3.is_some(), "P3 should fire at exactly 3 sites");
        if let WarnEarlyPattern::SharedMutableAt3PlusSites { site_count, .. } = &p3.unwrap().pattern
        {
            assert_eq!(*site_count, 3);
        }
    }

    #[test]
    fn p3_above_threshold() {
        // 10 mutable sites → P3 detected with site_count=10.
        let kir = make_kir_with_binding(10);
        let facts = detect_warn_early(&kir);
        let p3 = facts.iter().find(|f| {
            matches!(
                &f.pattern,
                WarnEarlyPattern::SharedMutableAt3PlusSites { .. }
            )
        });
        assert!(p3.is_some());
        if let WarnEarlyPattern::SharedMutableAt3PlusSites { site_count, .. } = &p3.unwrap().pattern
        {
            assert_eq!(*site_count, 10);
        }
    }

    // --- P4 additional tests ---

    #[test]
    fn p4_multiple_self_fields() {
        // Struct with 2 direct Self fields → still one P4 fact.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Double".to_owned(),
            span: dummy_span(),
            fields: vec![
                KirStructFieldDef {
                    name: "left".to_owned(),
                    shape: FieldTypeShape::DirectNamed("Double".to_owned()),
                },
                KirStructFieldDef {
                    name: "right".to_owned(),
                    shape: FieldTypeShape::DirectNamed("Double".to_owned()),
                },
            ],
            known_debt_reason: None,
            known_debt_span: None,
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        let p4_count = facts
            .iter()
            .filter(|f| matches!(&f.pattern, WarnEarlyPattern::SelfReferentialStruct { .. }))
            .count();
        assert_eq!(
            p4_count, 1,
            "one P4 fact per struct regardless of field count"
        );
    }

    // --- Edge case tests ---

    #[test]
    fn empty_struct_no_detection() {
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Empty".to_owned(),
            span: dummy_span(),
            fields: Vec::new(),
            known_debt_reason: None,
            known_debt_span: None,
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert!(facts.is_empty(), "empty struct should produce no facts");
    }

    #[test]
    fn known_debt_empty_struct() {
        // Struct with known_debt but no problematic fields → no facts.
        let kir = make_kir_with_structs(vec![KirStructDef {
            name: "Safe".to_owned(),
            span: dummy_span(),
            fields: vec![KirStructFieldDef {
                name: "value".to_owned(),
                shape: FieldTypeShape::Other,
            }],
            known_debt_reason: Some("precautionary".to_owned()),
            known_debt_span: Some(dummy_span()),
            known_debt_parse_error: None,
        }]);
        let facts = detect_warn_early(&kir);
        assert!(facts.is_empty(), "no problematic fields means no facts");
    }
}
