/// Greedy pass for the migration solver.
///
/// For each `Undecided` binding:
/// 1. Local-only, no sharing needed → PlainOwned
/// 2. Read-only shared, needs Send → ArcShared
/// 3. Read-only shared, no Send → RcShared
/// 4. Mutably shared ≤2 sites, no Send → RcMutShared
/// 5. Mutably shared, needs Send → ArcMutShared
/// 6. Mutably shared >2 sites, conflict constraints → K0080
///
/// The greedy pass does NOT backtrack. Each binding is resolved independently.
use std::time::Instant;

use kobo_errors::KErrorCode;
use kobo_ir::{
    FileId, Kir, KirNodeId, KoboSpan, NodeKind, OwnershipTier, TierDecision, TierReason,
    TransformBindingFacts,
};

/// Configuration for the greedy solver pass.
#[derive(Clone, Debug)]
pub struct GreedyConfig {
    pub solver_cluster_limit: usize,
    pub solver_budget_seconds: f64,
    pub mutable_sites_threshold: usize,
}

impl Default for GreedyConfig {
    fn default() -> Self {
        Self {
            solver_cluster_limit: 2048,
            solver_budget_seconds: 5.0,
            mutable_sites_threshold: 2,
        }
    }
}

/// Result of the greedy pass.
pub struct GreedyPassResult {
    pub resolved: Vec<TierDecision>,
    pub unresolved: Vec<KirNodeId>,
    pub diagnostics: Vec<GreedyDiagnostic>,
    pub stats: GreedyStats,
}

/// A diagnostic emitted by the greedy pass.
#[derive(Clone, Debug)]
pub struct GreedyDiagnostic {
    pub code: KErrorCode,
    pub binding_name: String,
    pub span: KoboSpan,
    pub message: String,
}

/// Statistics from the greedy pass.
#[derive(Clone, Debug, Default)]
pub struct GreedyStats {
    pub total_undecided: usize,
    pub resolved_count: usize,
    pub unresolved_count: usize,
    pub k0080_count: usize,
    pub elapsed_ms: u64,
}

/// Run the greedy tier resolution pass over KIR.
pub fn greedy_resolve(kir: &Kir, config: &GreedyConfig) -> GreedyPassResult {
    let start = Instant::now();

    let facts = kir.transform_facts();
    let undecided: Vec<&TransformBindingFacts> = facts
        .bindings
        .iter()
        .filter(|b| {
            kir.get_node(b.node)
                .map(|n| n.ownership == OwnershipTier::Undecided && n.kind == NodeKind::Decl)
                .unwrap_or(false)
        })
        .collect();

    let total_undecided = undecided.len();

    // Check cluster limit.
    if total_undecided > config.solver_cluster_limit {
        let elapsed = start.elapsed().as_millis() as u64;
        return GreedyPassResult {
            resolved: Vec::new(),
            unresolved: undecided.iter().map(|b| b.node).collect(),
            diagnostics: vec![GreedyDiagnostic {
                code: KErrorCode::K0081,
                binding_name: String::new(),
                span: KoboSpan::new(0, 0, FileId(0)),
                message: format!(
                    "ownership cluster has {} bindings, exceeding limit of {}",
                    total_undecided, config.solver_cluster_limit
                ),
            }],
            stats: GreedyStats {
                total_undecided,
                resolved_count: 0,
                unresolved_count: total_undecided,
                k0080_count: 0,
                elapsed_ms: elapsed,
            },
        };
    }

    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    let mut diagnostics = Vec::new();
    let mut k0080_count = 0;

    for binding in &undecided {
        // Check time budget.
        let elapsed_secs = start.elapsed().as_secs_f64();
        if elapsed_secs > config.solver_budget_seconds {
            // Budget exceeded — push remaining as unresolved.
            diagnostics.push(GreedyDiagnostic {
                code: KErrorCode::K0082,
                binding_name: binding.binding_name.clone(),
                span: binding.span,
                message: format!(
                    "solver budget of {:.1}s exceeded after resolving {} bindings",
                    config.solver_budget_seconds,
                    resolved.len()
                ),
            });
            // Collect remaining unresolved.
            unresolved.push(binding.node);
            continue;
        }

        match resolve_single_binding(binding, config.mutable_sites_threshold) {
            SingleResult::Resolved(tier, reason) => {
                resolved.push(TierDecision {
                    node: binding.node,
                    tier,
                    reason,
                    annotate: true,
                });
            }
            SingleResult::Conflict(message) => {
                k0080_count += 1;
                diagnostics.push(GreedyDiagnostic {
                    code: KErrorCode::K0080,
                    binding_name: binding.binding_name.clone(),
                    span: binding.span,
                    message,
                });
                unresolved.push(binding.node);
            }
            SingleResult::Unresolved => {
                unresolved.push(binding.node);
            }
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    // Post-greedy consistency check: demote bindings whose tiers conflict
    // with co-scoped resolved bindings under sharing/send constraints.
    let inconsistent = find_inconsistent_greedy_pairs(&resolved, kir);
    for node_id in &inconsistent {
        resolved.retain(|d| d.node != *node_id);
        unresolved.push(*node_id);
    }

    let resolved_count = resolved.len();
    let unresolved_count = unresolved.len();
    GreedyPassResult {
        resolved,
        unresolved,
        diagnostics,
        stats: GreedyStats {
            total_undecided,
            resolved_count,
            unresolved_count,
            k0080_count,
            elapsed_ms: elapsed,
        },
    }
}

enum SingleResult {
    Resolved(OwnershipTier, TierReason),
    Conflict(String),
    Unresolved,
}

fn resolve_single_binding(
    binding: &TransformBindingFacts,
    mutable_sites_threshold: usize,
) -> SingleResult {
    let sf = &binding.shared_facts;
    let requires_send = binding_requires_send(binding);

    // Rule 1: Not shared at all → PlainOwned.
    if !sf.needs_sharing && !sf.has_escape {
        return SingleResult::Resolved(OwnershipTier::PlainOwned, TierReason::LocalOnly);
    }

    // Rule 2: Read-only shared.
    if sf.needs_sharing && !sf.needs_mutable_wrapper {
        if requires_send {
            return SingleResult::Resolved(
                OwnershipTier::ArcShared,
                TierReason::SendRequiredShared,
            );
        }
        return SingleResult::Resolved(
            OwnershipTier::RcShared,
            TierReason::ReadOnlyShared {
                read_sites: sf.read_sites,
                sequential_only: sf.sequential_read_only,
            },
        );
    }

    // Rule 3-5: Mutably shared.
    if sf.needs_sharing && sf.needs_mutable_wrapper {
        if requires_send {
            return SingleResult::Resolved(
                OwnershipTier::ArcMutShared,
                TierReason::MutableSharedLastResort,
            );
        }
        if sf.mutable_sites <= mutable_sites_threshold {
            return SingleResult::Resolved(
                OwnershipTier::RcMutShared,
                TierReason::MutableSharedLastResort,
            );
        }
        // >2 mutable sites without Send → structural conflict.
        return SingleResult::Conflict(format!(
            "binding '{}' has {} mutable sharing sites — requires architectural decision",
            binding.binding_name, sf.mutable_sites,
        ));
    }

    // Escape but no sharing: needs box or remains unresolved.
    if sf.has_escape && sf.box_reason.is_some() {
        if let Some(box_reason) = sf.box_reason {
            return SingleResult::Resolved(
                OwnershipTier::BoxOwned,
                TierReason::HeapStable(box_reason),
            );
        }
    }

    SingleResult::Unresolved
}

fn binding_requires_send(binding: &TransformBindingFacts) -> bool {
    binding.shared_facts.needs_send || binding.async_shared || binding.is_async
}

/// Find resolved bindings whose tiers are inconsistent with co-scoped bindings.
///
/// If binding A was resolved to RcShared but co-scoped binding B needs Send
/// (and thus ArcShared), A's RcShared is inconsistent because in the same
/// scope, sharing propagation would require both to be thread-safe.
///
/// Returns node IDs that should be demoted from resolved to unresolved.
fn find_inconsistent_greedy_pairs(resolved: &[TierDecision], kir: &Kir) -> Vec<KirNodeId> {
    use std::collections::{BTreeMap, BTreeSet};

    let facts = kir.transform_facts();

    // Build a map: node_id → (tier, needs_send).
    let mut info: BTreeMap<KirNodeId, (OwnershipTier, bool)> = BTreeMap::new();
    for decision in resolved {
        let needs_send = facts
            .bindings
            .iter()
            .find(|b| b.node == decision.node)
            .map(binding_requires_send)
            .unwrap_or(false);
        info.insert(decision.node, (decision.tier, needs_send));
    }

    // Group resolved bindings by scope depth.
    let mut scope_groups: BTreeMap<usize, Vec<KirNodeId>> = BTreeMap::new();
    for binding in &facts.bindings {
        if info.contains_key(&binding.node) {
            scope_groups
                .entry(binding.decl_scope_depth)
                .or_default()
                .push(binding.node);
        }
    }

    let mut inconsistent = BTreeSet::new();

    for group in scope_groups.values() {
        if group.len() < 2 {
            continue;
        }
        // If any binding in the group needs Send, all shared bindings should
        // be thread-safe. If one was resolved as Rc but another needs Send,
        // the Rc one is inconsistent.
        let any_needs_send = group
            .iter()
            .any(|id| info.get(id).map(|(_, ns)| *ns).unwrap_or(false));
        if any_needs_send {
            for &id in group {
                if let Some(&(tier, _)) = info.get(&id) {
                    if tier == OwnershipTier::RcShared || tier == OwnershipTier::RcMutShared {
                        inconsistent.insert(id);
                    }
                }
            }
        }
    }

    inconsistent.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{
        BindingUsage, KirNode, KirNodeId, KoboAstNodeId, KoboSpan, NodeKind, SharedBindingFacts,
        TransformFacts,
    };

    fn make_kir_with_bindings(bindings: Vec<TransformBindingFacts>) -> Kir {
        let mut nodes: Vec<KirNode> = bindings
            .iter()
            .map(|b| KirNode {
                id: b.node,
                kind: NodeKind::Decl,
                ast_id: Some(b.ast_id),
                ownership: OwnershipTier::Undecided,
                resource_kind: None,
                cfg_block: None,
                span: b.span,
                decl_id: None,
            })
            .collect();

        // Ensure we have at least one node.
        if nodes.is_empty() {
            nodes.push(KirNode {
                id: KirNodeId(0),
                kind: NodeKind::Decl,
                ast_id: None,
                ownership: OwnershipTier::Undecided,
                resource_kind: None,
                cfg_block: None,
                span: KoboSpan::new(0, 0, FileId(0)),
                decl_id: None,
            });
        }

        let facts = TransformFacts {
            bindings,
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        };

        let mut kir = Kir::from_nodes(nodes);
        kir.set_transform_facts(facts);
        kir
    }

    fn make_binding(
        id: u32,
        name: &str,
        needs_sharing: bool,
        needs_mutable: bool,
        needs_send: bool,
        mutable_sites: usize,
    ) -> TransformBindingFacts {
        TransformBindingFacts {
            node: KirNodeId(id),
            ast_id: KoboAstNodeId(id),
            binding_name: name.to_owned(),
            span: KoboSpan::new(0, 0, FileId(0)),
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
                mutation_required: needs_mutable,
                escape_floor: None,
                borrow_sites: Vec::new(),
                read_sites: if needs_sharing && !needs_mutable {
                    2
                } else {
                    0
                },
                mutable_sites,
                has_escape: false,
                needs_sharing,
                needs_mutable_wrapper: needs_mutable,
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
            ref_returning_read_spans: Vec::new(),
        }
    }

    #[test]
    fn greedy_resolves_local_only_to_plain_owned() {
        let binding = make_binding(1, "buf", false, false, false, 0);
        let kir = make_kir_with_bindings(vec![binding]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.resolved_count, 1);
        assert_eq!(result.resolved[0].tier, OwnershipTier::PlainOwned);
    }

    #[test]
    fn greedy_resolves_shared_read_to_rc() {
        let binding = make_binding(1, "data", true, false, false, 0);
        let kir = make_kir_with_bindings(vec![binding]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.resolved_count, 1);
        assert_eq!(result.resolved[0].tier, OwnershipTier::RcShared);
    }

    #[test]
    fn greedy_resolves_send_shared_to_arc() {
        let binding = make_binding(1, "data", true, false, true, 0);
        let kir = make_kir_with_bindings(vec![binding]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.resolved_count, 1);
        assert_eq!(result.resolved[0].tier, OwnershipTier::ArcShared);
    }

    #[test]
    fn greedy_resolves_mutable_shared_to_rc_mut() {
        let binding = make_binding(1, "state", true, true, false, 2);
        let kir = make_kir_with_bindings(vec![binding]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.resolved_count, 1);
        assert_eq!(result.resolved[0].tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn greedy_resolves_send_mutable_to_arc_mut() {
        let binding = make_binding(1, "state", true, true, true, 3);
        let kir = make_kir_with_bindings(vec![binding]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.resolved_count, 1);
        assert_eq!(result.resolved[0].tier, OwnershipTier::ArcMutShared);
    }

    #[test]
    fn greedy_emits_k0080_on_high_mutable_conflict() {
        let binding = make_binding(1, "data", true, true, false, 5);
        let kir = make_kir_with_bindings(vec![binding]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.k0080_count, 1);
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == KErrorCode::K0080);
        assert!(diag.is_some());
    }

    #[test]
    fn greedy_respects_cluster_limit() {
        let bindings: Vec<TransformBindingFacts> = (0..30)
            .map(|i| make_binding(i, &format!("b{i}"), false, false, false, 0))
            .collect();
        let kir = make_kir_with_bindings(bindings);
        let config = GreedyConfig {
            solver_cluster_limit: 20,
            ..Default::default()
        };
        let result = greedy_resolve(&kir, &config);
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == KErrorCode::K0081);
        assert!(diag.is_some());
        assert_eq!(result.stats.unresolved_count, 30);
    }

    #[test]
    fn greedy_empty_input_no_crash() {
        let kir = make_kir_with_bindings(vec![]);
        let result = greedy_resolve(&kir, &GreedyConfig::default());
        assert_eq!(result.stats.total_undecided, 0);
        assert_eq!(result.stats.resolved_count, 0);
    }
}
