use kobo_errors::Severity;
/// Pinpoint the exact binding and await point causing a non-Send future.
///
/// When a binding with tier Rc*/RefCell crosses a spawn boundary,
/// the generated tokio::spawn will fail with "future is not Send."
///
/// Kobo's diagnostic replaces this with:
///   error[K0061]: future requires Send but `state` cannot cross thread boundary
///
/// This requires:
/// 1. Knowing which bindings are captured by the spawn block
/// 2. Knowing which bindings are !Send (Rc, RefCell, non-Send user types)
/// 3. Finding the .await points that cause the capture to span across suspend
use kobo_ir::{Kir, KirNodeId, KoboSpan, NodeKind, OwnershipTier, TransformFacts};

/// Represents a spawn site in the source.
#[derive(Clone, Debug)]
pub struct SpawnSite {
    pub span: KoboSpan,
    pub captured_bindings: Vec<KirNodeId>,
    pub await_points: Vec<KoboSpan>,
}

/// A diagnostic for a non-Send binding crossing a spawn boundary.
#[derive(Clone, Debug)]
pub struct SendDiagnostic {
    /// Severity — K0061 is always an error (not a warning).
    pub severity: Severity,
    /// The binding name that cannot cross the thread boundary.
    pub binding_name: String,
    /// The wrapper type string, e.g. "Rc<RefCell<State>>".
    pub wrapper_type: String,
    /// Where the spawn block starts.
    pub spawn_span: KoboSpan,
    /// Where the binding is declared.
    pub binding_span: KoboSpan,
    /// The .await that causes the binding to be held across a suspend point.
    pub await_span: Option<KoboSpan>,
    /// Suggestion for fixing the issue.
    pub suggestion: String,
}

/// Determine if an ownership tier is !Send.
fn is_not_send(tier: OwnershipTier) -> bool {
    matches!(tier, OwnershipTier::RcShared | OwnershipTier::RcMutShared)
}

/// Human-readable wrapper type name for an ownership tier.
fn wrapper_type_label(tier: OwnershipTier) -> &'static str {
    match tier {
        OwnershipTier::RcShared => "Rc<T>",
        OwnershipTier::RcMutShared => "Rc<RefCell<T>>",
        OwnershipTier::ArcShared => "Arc<T>",
        OwnershipTier::ArcMutShared => "Arc<RwLock<T>>",
        OwnershipTier::BoxOwned => "Box<T>",
        OwnershipTier::PlainOwned => "T",
        OwnershipTier::Scoped => "ScopedHandle<T>",
        OwnershipTier::Undecided => "?",
    }
}

/// Suggestion for fixing a non-Send binding.
fn suggest_fix(tier: OwnershipTier) -> String {
    match tier {
        OwnershipTier::RcShared => {
            "Change to Arc<T> — add #[kobo::async_shared] or use Arc::new() directly".to_owned()
        }
        OwnershipTier::RcMutShared => {
            "Change to Arc<RwLock<T>> — add #[kobo::async_shared] to enable thread-safe sharing"
                .to_owned()
        }
        _ => "Ensure the type implements Send".to_owned(),
    }
}

/// Analyze spawn sites for Send violations.
///
/// For each spawn site, check each captured binding's ownership tier.
/// If the tier is !Send, produce a diagnostic.
pub fn analyze_send_violations(
    spawn_sites: &[SpawnSite],
    facts: &TransformFacts,
    kir: &Kir,
) -> Vec<SendDiagnostic> {
    let mut diagnostics = Vec::new();

    for site in spawn_sites {
        for &binding_id in &site.captured_bindings {
            let Some(node) = kir.get_node(binding_id) else {
                continue;
            };

            if node.kind != NodeKind::Decl {
                continue;
            }

            if !is_not_send(node.ownership) {
                continue;
            }

            // Find the binding name from transform facts.
            let binding_name = facts
                .bindings
                .iter()
                .find(|b| b.node == binding_id)
                .map(|b| b.binding_name.clone())
                .unwrap_or_else(|| format!("_node{}", binding_id.0));

            let wrapper_type = wrapper_type_label(node.ownership).to_owned();
            let suggestion = suggest_fix(node.ownership);

            // Pick the first .await point as the primary cause.
            let await_span = site.await_points.first().copied();

            diagnostics.push(SendDiagnostic {
                severity: Severity::Error,
                binding_name,
                wrapper_type,
                spawn_span: site.span,
                binding_span: node.span,
                await_span,
                suggestion,
            });
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{
        BindingUsage, FileId, KirNode, KirNodeId, KoboAstNodeId, KoboSpan, SharedBindingFacts,
        TransformBindingFacts, TransformFacts,
    };

    fn make_binding_facts(id: u32, name: &str) -> TransformBindingFacts {
        TransformBindingFacts {
            node: KirNodeId(id),
            ast_id: KoboAstNodeId(id),
            binding_name: name.to_owned(),
            span: KoboSpan::new(10, 20, FileId(0)),
            resource_kind: None,
            hint: None,
            hint_span: None,
            is_copy_known: false,
            is_generic: false,
            is_async: false,
            async_shared: false,
            usage: BindingUsage {
                declaration: KoboSpan::new(10, 20, FileId(0)),
                uses: Vec::new(),
            },
            shared_facts: SharedBindingFacts {
                node_id: KirNodeId(id),
                mutation_required: false,
                escape_floor: None,
                borrow_sites: Vec::new(),
                read_sites: 0,
                mutable_sites: 0,
                has_escape: false,
                needs_sharing: false,
                needs_mutable_wrapper: false,
                needs_send: false,
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
            ref_returning_read_spans: Vec::new(),
        }
    }

    fn make_kir_with_tier(id: u32, tier: OwnershipTier) -> Kir {
        let node = KirNode {
            id: KirNodeId(id),
            kind: NodeKind::Decl,
            ast_id: Some(KoboAstNodeId(id)),
            ownership: tier,
            resource_kind: None,
            cfg_block: None,
            span: KoboSpan::new(10, 20, FileId(0)),
            decl_id: None,
        };
        Kir::from_nodes(vec![node])
    }

    #[test]
    fn send_diagnostic_includes_binding_name() {
        let mut kir = make_kir_with_tier(1, OwnershipTier::RcMutShared);
        let facts = TransformFacts {
            bindings: vec![make_binding_facts(1, "state")],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 200, FileId(0)),
            captured_bindings: vec![KirNodeId(1)],
            await_points: vec![KoboSpan::new(150, 160, FileId(0))],
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].binding_name, "state");
        assert!(diagnostics[0].wrapper_type.contains("Rc"));
        assert!(diagnostics[0].await_span.is_some());
    }

    #[test]
    fn no_send_diagnostic_for_arc() {
        let mut kir = make_kir_with_tier(1, OwnershipTier::ArcShared);
        let facts = TransformFacts {
            bindings: vec![make_binding_facts(1, "state")],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 200, FileId(0)),
            captured_bindings: vec![KirNodeId(1)],
            await_points: vec![KoboSpan::new(150, 160, FileId(0))],
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert!(diagnostics.is_empty(), "Arc is Send — no diagnostic");
    }

    #[test]
    fn send_diagnostic_for_rc_shared() {
        let mut kir = make_kir_with_tier(1, OwnershipTier::RcShared);
        let facts = TransformFacts {
            bindings: vec![make_binding_facts(1, "data")],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 200, FileId(0)),
            captured_bindings: vec![KirNodeId(1)],
            await_points: Vec::new(), // no await points
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].binding_name, "data");
        assert!(diagnostics[0].await_span.is_none());
    }

    #[test]
    fn no_diagnostic_for_plain_owned() {
        let mut kir = make_kir_with_tier(1, OwnershipTier::PlainOwned);
        let facts = TransformFacts {
            bindings: vec![make_binding_facts(1, "buf")],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 200, FileId(0)),
            captured_bindings: vec![KirNodeId(1)],
            await_points: vec![KoboSpan::new(150, 160, FileId(0))],
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert!(diagnostics.is_empty(), "PlainOwned is Send");
    }

    #[test]
    fn suggestion_for_rc_is_arc() {
        let diag = suggest_fix(OwnershipTier::RcShared);
        assert!(diag.contains("Arc"));
    }

    #[test]
    fn suggestion_for_rc_mut_is_rwlock() {
        let diag = suggest_fix(OwnershipTier::RcMutShared);
        assert!(diag.contains("RwLock"));
    }

    // ─── BUG-04 test: severity field ───

    #[test]
    fn send_diagnostic_severity_is_error() {
        let mut kir = make_kir_with_tier(1, OwnershipTier::RcShared);
        let facts = TransformFacts {
            bindings: vec![make_binding_facts(1, "state")],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 200, FileId(0)),
            captured_bindings: vec![KirNodeId(1)],
            await_points: vec![KoboSpan::new(150, 160, FileId(0))],
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            Severity::Error,
            "K0061 should be an error, not a warning"
        );
    }

    // ─── v0.8 edge-case tests ───

    /// K0061 always Error for RcMutShared too.
    #[test]
    fn k0061_rc_mut_shared_is_error() {
        let mut kir = make_kir_with_tier(1, OwnershipTier::RcMutShared);
        let facts = TransformFacts {
            bindings: vec![make_binding_facts(1, "shared_state")],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 200, FileId(0)),
            captured_bindings: vec![KirNodeId(1)],
            await_points: vec![],
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].wrapper_type.contains("Rc"));
    }

    /// All Send-safe tiers produce zero diagnostics.
    #[test]
    fn all_send_safe_tiers_no_diagnostic() {
        for tier in [
            OwnershipTier::PlainOwned,
            OwnershipTier::BoxOwned,
            OwnershipTier::ArcShared,
            OwnershipTier::ArcMutShared,
        ] {
            let mut kir = make_kir_with_tier(1, tier);
            let facts = TransformFacts {
                bindings: vec![make_binding_facts(1, "data")],
                usages: Vec::new(),
                shared_facts: Vec::new(),
                hint_conflicts: Vec::new(),
            };
            kir.set_transform_facts(facts.clone());

            let spawn_sites = vec![SpawnSite {
                span: KoboSpan::new(0, 100, FileId(0)),
                captured_bindings: vec![KirNodeId(1)],
                await_points: vec![KoboSpan::new(50, 60, FileId(0))],
            }];

            let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
            assert!(
                diagnostics.is_empty(),
                "{tier:?} is Send — should not trigger K0061"
            );
        }
    }

    /// Multiple !Send bindings in one spawn → one diagnostic per binding.
    #[test]
    fn multiple_non_send_bindings_in_one_spawn() {
        let nodes = vec![
            KirNode {
                id: KirNodeId(10),
                kind: NodeKind::Decl,
                ast_id: Some(KoboAstNodeId(10)),
                ownership: OwnershipTier::RcShared,
                resource_kind: None,
                cfg_block: None,
                span: KoboSpan::new(10, 20, FileId(0)),
                decl_id: None,
            },
            KirNode {
                id: KirNodeId(20),
                kind: NodeKind::Decl,
                ast_id: Some(KoboAstNodeId(20)),
                ownership: OwnershipTier::RcMutShared,
                resource_kind: None,
                cfg_block: None,
                span: KoboSpan::new(30, 40, FileId(0)),
                decl_id: None,
            },
        ];
        let mut kir = Kir::from_nodes(nodes);
        let facts = TransformFacts {
            bindings: vec![
                make_binding_facts(10, "cache"),
                make_binding_facts(20, "state"),
            ],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        kir.set_transform_facts(facts.clone());

        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(100, 300, FileId(0)),
            captured_bindings: vec![KirNodeId(10), KirNodeId(20)],
            await_points: vec![],
        }];

        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert_eq!(
            diagnostics.len(),
            2,
            "expected 2 diagnostics for 2 !Send bindings"
        );
        let names: Vec<&str> = diagnostics
            .iter()
            .map(|d| d.binding_name.as_str())
            .collect();
        assert!(names.contains(&"cache"));
        assert!(names.contains(&"state"));
    }

    /// Empty spawn sites → zero diagnostics.
    #[test]
    fn no_spawn_sites_no_diagnostics() {
        let facts = TransformFacts {
            bindings: vec![],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        let kir = Kir::from_nodes(vec![]);
        let diagnostics = analyze_send_violations(&[], &facts, &kir);
        assert!(diagnostics.is_empty());
    }

    /// Spawn site with unknown binding id → silently skipped.
    #[test]
    fn unknown_binding_id_silently_skipped() {
        let kir = Kir::from_nodes(vec![]);
        let facts = TransformFacts {
            bindings: vec![],
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };
        let spawn_sites = vec![SpawnSite {
            span: KoboSpan::new(0, 100, FileId(0)),
            captured_bindings: vec![KirNodeId(999)],
            await_points: vec![],
        }];
        let diagnostics = analyze_send_violations(&spawn_sites, &facts, &kir);
        assert!(
            diagnostics.is_empty(),
            "unknown binding should be silently skipped"
        );
    }

    /// is_not_send for all 8 tiers.
    #[test]
    fn is_not_send_truth_table() {
        assert!(!is_not_send(OwnershipTier::PlainOwned));
        assert!(!is_not_send(OwnershipTier::BoxOwned));
        assert!(is_not_send(OwnershipTier::RcShared));
        assert!(!is_not_send(OwnershipTier::ArcShared));
        assert!(!is_not_send(OwnershipTier::ArcMutShared));
        assert!(is_not_send(OwnershipTier::RcMutShared));
        assert!(!is_not_send(OwnershipTier::Scoped));
        assert!(!is_not_send(OwnershipTier::Undecided));
    }
}
