/// 6 migration patterns with step-by-step instructions.
///
/// Patterns:
/// 1. Rc→Arc escalation — "Your binding crossed a spawn boundary"
/// 2. RefCell→RwLock — "Your binding is mutably shared in async context"
/// 3. Clone elimination — "This clone can be replaced with a borrow"
/// 4. Lifetime insertion — "This clone can be replaced with &'a T"
/// 5. Arc de-escalation — "This Arc is only used in single-threaded context"
/// 6. Box→PlainOwned — "This Box is unnecessary; value fits on stack"
use kobo_ir::{Kir, KoboSpan, NodeKind, OwnershipTier, TierDecision};
use serde::Serialize;

/// A detected migration pattern for a specific binding.
#[derive(Clone, Debug, Serialize)]
pub struct MigrationPattern {
    /// Pattern name (one of the 6 canonical patterns).
    pub name: &'static str,
    /// The binding this pattern applies to.
    pub binding_name: String,
    /// Current ownership tier of the binding.
    pub current_tier: OwnershipTier,
    /// Suggested replacement tier.
    pub suggested_tier: OwnershipTier,
    /// Human-readable explanation of why.
    pub explanation: String,
    /// Risk level of this migration.
    pub risk: PatternRisk,
    /// Source location.
    pub span: KoboSpan,
}

/// Risk level for a migration pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum PatternRisk {
    /// Mechanical change, no behavior change.
    Safe,
    /// Needs human review — may change semantics.
    MayChangeBehavior,
}

/// Detect migration patterns from KIR and tier decisions.
///
/// This function looks for cases where the chosen tier could be improved:
/// - Arc that doesn't need Send → de-escalate to Rc
/// - Rc that crosses spawn → escalate to Arc
/// - Rc<RefCell<T>> in async → suggest RwLock
/// - Clone that could be a borrow
/// - Clone that could be a lifetime parameter
/// - Box that could be PlainOwned (stack allocation)
pub fn detect_migration_patterns(kir: &Kir, decisions: &[TierDecision]) -> Vec<MigrationPattern> {
    let mut patterns = Vec::new();
    let facts = kir.transform_facts();

    for decision in decisions {
        let Some(node) = kir.get_node(decision.node) else {
            continue;
        };
        if node.kind != NodeKind::Decl {
            continue;
        }

        // Find the binding facts for this node.
        let binding = facts.bindings.iter().find(|b| b.node == decision.node);
        let binding_name = binding
            .map(|b| b.binding_name.clone())
            .unwrap_or_else(|| format!("node_{}", decision.node.0));

        let sf = binding.map(|b| &b.shared_facts);

        // Pattern 1: Rc → Arc escalation (binding crossed spawn boundary).
        if decision.tier == OwnershipTier::RcShared {
            if let Some(sf) = sf {
                if sf.needs_send {
                    patterns.push(MigrationPattern {
                        name: "Rc→Arc escalation",
                        binding_name: binding_name.clone(),
                        current_tier: OwnershipTier::RcShared,
                        suggested_tier: OwnershipTier::ArcShared,
                        explanation: "Your binding crossed a spawn boundary — Rc is not Send"
                            .to_owned(),
                        risk: PatternRisk::Safe,
                        span: node.span,
                    });
                }
            }
        }

        // Pattern 2: RefCell → RwLock (mutably shared in async context).
        if decision.tier == OwnershipTier::RcMutShared {
            if let Some(sf) = sf {
                if sf.needs_send {
                    patterns.push(MigrationPattern {
                        name: "RefCell→RwLock",
                        binding_name: binding_name.clone(),
                        current_tier: OwnershipTier::RcMutShared,
                        suggested_tier: OwnershipTier::ArcMutShared,
                        explanation:
                            "Your binding is mutably shared in async context — RefCell is not Send"
                                .to_owned(),
                        risk: PatternRisk::MayChangeBehavior,
                        span: node.span,
                    });
                }
            }
        }

        // Pattern 3: Clone elimination (clone can be replaced with borrow).
        if let Some(binding) = binding {
            if binding.clone_elision.is_some() && !binding.plain_clone_alias {
                patterns.push(MigrationPattern {
                    name: "Clone elimination",
                    binding_name: binding_name.clone(),
                    current_tier: decision.tier,
                    suggested_tier: decision.tier,
                    explanation: "This clone can be replaced with a borrow".to_owned(),
                    risk: PatternRisk::Safe,
                    span: node.span,
                });
            }
        }

        // Pattern 4: Lifetime insertion (clone can be replaced with &'a T).
        if let Some(binding) = binding {
            if binding.elision_fallback.is_some() {
                patterns.push(MigrationPattern {
                    name: "Lifetime insertion",
                    binding_name: binding_name.clone(),
                    current_tier: decision.tier,
                    suggested_tier: decision.tier,
                    explanation: "This clone can be replaced with &'a T".to_owned(),
                    risk: PatternRisk::MayChangeBehavior,
                    span: node.span,
                });
            }
        }

        // Pattern 5: Arc de-escalation (Arc but no Send needed).
        if decision.tier == OwnershipTier::ArcShared {
            if let Some(sf) = sf {
                if !sf.needs_send {
                    patterns.push(MigrationPattern {
                        name: "Arc de-escalation",
                        binding_name: binding_name.clone(),
                        current_tier: OwnershipTier::ArcShared,
                        suggested_tier: OwnershipTier::RcShared,
                        explanation:
                            "This Arc is only used in single-threaded context — Rc is cheaper"
                                .to_owned(),
                        risk: PatternRisk::Safe,
                        span: node.span,
                    });
                }
            }
        }

        // Pattern 6: Box → PlainOwned (unnecessary heap allocation).
        if decision.tier == OwnershipTier::BoxOwned {
            if let Some(sf) = sf {
                if !sf.has_escape && !sf.needs_sharing {
                    patterns.push(MigrationPattern {
                        name: "Box→PlainOwned",
                        binding_name: binding_name.clone(),
                        current_tier: OwnershipTier::BoxOwned,
                        suggested_tier: OwnershipTier::PlainOwned,
                        explanation: "This Box is unnecessary; value fits on stack".to_owned(),
                        risk: PatternRisk::Safe,
                        span: node.span,
                    });
                }
            }
        }
    }

    patterns
}

/// Format migration patterns for human-readable output.
pub fn format_patterns(patterns: &[MigrationPattern]) -> String {
    if patterns.is_empty() {
        return "No migration patterns detected.".to_owned();
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{} migration pattern(s) detected:\n\n",
        patterns.len()
    ));

    for (i, pattern) in patterns.iter().enumerate() {
        let risk_label = match pattern.risk {
            PatternRisk::Safe => "safe",
            PatternRisk::MayChangeBehavior => "needs review",
        };

        output.push_str(&format!(
            "  {}. [{}] '{}': {:?} → {:?}\n     {} ({})\n\n",
            i + 1,
            pattern.name,
            pattern.binding_name,
            pattern.current_tier,
            pattern.suggested_tier,
            pattern.explanation,
            risk_label,
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{
        BindingUsage, FileId, KirNode, KirNodeId, KoboAstNodeId, KoboSpan, SharedBindingFacts,
        TierReason, TransformBindingFacts, TransformFacts,
    };

    fn make_kir_and_decisions(
        bindings: Vec<(TransformBindingFacts, OwnershipTier, TierReason)>,
    ) -> (Kir, Vec<TierDecision>) {
        let nodes: Vec<KirNode> = bindings
            .iter()
            .map(|(b, tier, _)| KirNode {
                id: b.node,
                kind: NodeKind::Decl,
                ast_id: Some(b.ast_id),
                ownership: *tier,
                resource_kind: None,
                cfg_block: None,
                span: b.span,
                decl_id: None,
            })
            .collect();

        let decisions: Vec<TierDecision> = bindings
            .iter()
            .map(|(b, tier, reason)| TierDecision {
                node: b.node,
                tier: *tier,
                reason: reason.clone(),
                annotate: true,
            })
            .collect();

        let facts = TransformFacts {
            bindings: bindings.into_iter().map(|(b, _, _)| b).collect(),
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };

        let mut kir = Kir::from_nodes(nodes);
        kir.set_transform_facts(facts);
        (kir, decisions)
    }

    fn make_binding(
        id: u32,
        name: &str,
        needs_sharing: bool,
        needs_send: bool,
        has_escape: bool,
    ) -> TransformBindingFacts {
        TransformBindingFacts {
            node: KirNodeId(id),
            ast_id: KoboAstNodeId(id),
            binding_name: name.to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            resource_kind: None,
            hint: None,
            hint_span: None,
            is_copy_known: false,
            is_generic: false,
            is_async: false,
            async_shared: false,
            usage: BindingUsage {
                declaration: KoboSpan::new(0, 0, FileId(0)),
                uses: Vec::new(),
            },
            shared_facts: SharedBindingFacts {
                node_id: KirNodeId(id),
                mutation_required: false,
                escape_floor: None,
                borrow_sites: Vec::new(),
                read_sites: 0,
                mutable_sites: 0,
                has_escape,
                needs_sharing,
                needs_mutable_wrapper: false,
                needs_send,
                live_borrow_at_move: false,
                sequential_read_only: false,
                box_reason: None,
            },
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
            decl_scope_depth: 0,
            method_read_spans: Vec::new(),
            ref_returning_read_spans: Vec::new(),
        }
    }

    #[test]
    fn detect_arc_deescalation_pattern() {
        // Arc binding that doesn't need Send → should suggest Rc.
        let binding = make_binding(1, "data", true, false, false);
        let (kir, decisions) = make_kir_and_decisions(vec![(
            binding,
            OwnershipTier::ArcShared,
            TierReason::SendRequiredShared,
        )]);

        let patterns = detect_migration_patterns(&kir, &decisions);
        let deesc = patterns.iter().find(|p| p.name == "Arc de-escalation");
        assert!(deesc.is_some(), "should detect Arc de-escalation");
        assert_eq!(deesc.unwrap().suggested_tier, OwnershipTier::RcShared);
        assert_eq!(deesc.unwrap().risk, PatternRisk::Safe);
    }

    #[test]
    fn detect_rc_to_arc_escalation() {
        // Rc binding that needs Send → should suggest Arc.
        let binding = make_binding(1, "state", true, true, false);
        let (kir, decisions) = make_kir_and_decisions(vec![(
            binding,
            OwnershipTier::RcShared,
            TierReason::ReadOnlyShared {
                read_sites: 2,
                sequential_only: false,
            },
        )]);

        let patterns = detect_migration_patterns(&kir, &decisions);
        let esc = patterns.iter().find(|p| p.name == "Rc→Arc escalation");
        assert!(esc.is_some(), "should detect Rc→Arc escalation");
        assert_eq!(esc.unwrap().suggested_tier, OwnershipTier::ArcShared);
    }

    #[test]
    fn detect_box_to_plain_pattern() {
        // Box binding that doesn't escape or share → should suggest PlainOwned.
        let binding = make_binding(1, "buf", false, false, false);
        let (kir, decisions) = make_kir_and_decisions(vec![(
            binding,
            OwnershipTier::BoxOwned,
            TierReason::LocalOnly,
        )]);

        let patterns = detect_migration_patterns(&kir, &decisions);
        let debox = patterns.iter().find(|p| p.name == "Box→PlainOwned");
        assert!(debox.is_some(), "should detect Box→PlainOwned");
        assert_eq!(debox.unwrap().suggested_tier, OwnershipTier::PlainOwned);
    }

    #[test]
    fn no_pattern_when_tier_matches_needs() {
        // RcShared binding that actually needs sharing, no Send → no escalation pattern.
        let binding = make_binding(1, "data", true, false, false);
        let (kir, decisions) = make_kir_and_decisions(vec![(
            binding,
            OwnershipTier::RcShared,
            TierReason::ReadOnlyShared {
                read_sites: 2,
                sequential_only: false,
            },
        )]);

        let patterns = detect_migration_patterns(&kir, &decisions);
        assert!(
            patterns.is_empty(),
            "no patterns when tier matches actual needs: got {:?}",
            patterns.iter().map(|p| p.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn format_patterns_empty() {
        let output = format_patterns(&[]);
        assert_eq!(output, "No migration patterns detected.");
    }

    #[test]
    fn format_patterns_non_empty() {
        let patterns = vec![MigrationPattern {
            name: "Arc de-escalation",
            binding_name: "data".to_owned(),
            current_tier: OwnershipTier::ArcShared,
            suggested_tier: OwnershipTier::RcShared,
            explanation: "This Arc is only used in single-threaded context".to_owned(),
            risk: PatternRisk::Safe,
            span: KoboSpan::new(0, 10, FileId(0)),
        }];
        let output = format_patterns(&patterns);
        assert!(output.contains("Arc de-escalation"));
        assert!(output.contains("data"));
        assert!(output.contains("safe"));
    }
}
