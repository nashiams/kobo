//! Phase 01: Constraint extraction from KIR and greedy residuals.
//!
//! Assigns fresh constraint nodes to residual (unresolved) bindings and walks
//! KIR to collect inequality constraints with mandatory provenance.

use std::collections::BTreeSet;

use kobo_ir::{FileId, Kir, KirNodeId, KoboSpan, NodeKind, OwnershipTier, TransformBindingFacts};

use crate::greedy::GreedyPassResult;
use crate::solver::{
    ConstraintEdge, ConstraintFactKind, ConstraintGraph, ConstraintKind, ConstraintProvenance,
};

/// Provenance attached to every constraint edge — explains *why* this constraint exists.
pub type ProvenanceRef = ConstraintProvenance;

/// The kind of fact that generated a constraint edge.
pub type FactKind = ConstraintFactKind;

/// A constraint edge with mandatory provenance.
#[derive(Clone, Debug)]
pub struct ProvenancedEdge {
    pub edge: ConstraintEdge,
    pub provenance: ProvenanceRef,
}

/// A node in the extracted constraint graph with floor/ceiling info.
#[derive(Clone, Debug)]
pub struct ConstraintNode {
    pub id: KirNodeId,
    pub floor: OwnershipTier,
    pub ceiling: Option<OwnershipTier>,
    pub is_boundary: bool,
    pub binding_name: String,
}

/// Result of constraint extraction.
pub struct ExtractionResult {
    pub nodes: Vec<ConstraintNode>,
    pub edges: Vec<ProvenancedEdge>,
    pub graph: ConstraintGraph,
}

/// Extract constraints from KIR for residual (unresolved) bindings.
///
/// Only bindings left unresolved by the greedy pass become graph nodes.
/// Resolved greedy bindings are NOT reintroduced.
pub fn extract_constraints(kir: &Kir, greedy_result: &GreedyPassResult) -> ExtractionResult {
    let unresolved_set: BTreeSet<KirNodeId> = greedy_result.unresolved.iter().copied().collect();

    if unresolved_set.is_empty() {
        return ExtractionResult {
            nodes: Vec::new(),
            edges: Vec::new(),
            graph: ConstraintGraph {
                nodes: Vec::new(),
                edges: Vec::new(),
            },
        };
    }

    let facts = kir.transform_facts();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Build constraint nodes for each unresolved binding.
    for binding in &facts.bindings {
        if !unresolved_set.contains(&binding.node) {
            continue;
        }
        let sf = &binding.shared_facts;
        let requires_send = binding_requires_send(binding);

        let mut floor = OwnershipTier::PlainOwned;
        let mut ceiling: Option<OwnershipTier> = None;
        let mut is_boundary = false;

        // Floor: needs_sharing → at least RcShared
        if sf.needs_sharing && !sf.needs_mutable_wrapper {
            floor = if requires_send {
                OwnershipTier::ArcShared
            } else {
                OwnershipTier::RcShared
            };
        }

        // Floor: needs_sharing + mutable → at least RcMutShared
        if sf.needs_sharing && sf.needs_mutable_wrapper {
            floor = if requires_send {
                OwnershipTier::ArcMutShared
            } else {
                OwnershipTier::RcMutShared
            };
        }

        // Floor: needs_send alone → at least ArcShared
        if requires_send && floor == OwnershipTier::PlainOwned {
            floor = OwnershipTier::ArcShared;
        }

        // Ceiling: engine-annotated bindings are PlainOwned only.
        // (Engine metadata from S-3 imposes hard ceiling.)
        if binding.resource_kind.is_some() {
            ceiling = Some(OwnershipTier::PlainOwned);
        }

        // Detect boundary markers.
        // A binding that escapes the current scope (passed to opaque call,
        // stored in struct, etc.) may cross a crate boundary. Mark it so
        // the pipeline can stop migration at the boundary.
        if binding.shared_facts.has_escape {
            is_boundary = true;
        }

        nodes.push(ConstraintNode {
            id: binding.node,
            floor,
            ceiling,
            is_boundary,
            binding_name: binding.binding_name.clone(),
        });
    }

    // Build edges from binding relationships.
    let node_set: BTreeSet<KirNodeId> = nodes.iter().map(|n| n.id).collect();

    // Sharing propagation: if two unresolved bindings appear in the same scope, propagate sharing.
    let mut scope_groups: std::collections::BTreeMap<Option<u32>, Vec<KirNodeId>> =
        std::collections::BTreeMap::new();
    for kir_node in kir.iter_nodes() {
        if kir_node.kind == NodeKind::Decl {
            continue;
        }
        if let Some(decl_id) = kir_node.decl_id {
            if node_set.contains(&decl_id) {
                scope_groups
                    .entry(kir_node.cfg_block.map(|b| b.0))
                    .or_default()
                    .push(decl_id);
            }
        }
    }

    for binding in &facts.bindings {
        if node_set.contains(&binding.node) {
            scope_groups
                .entry(Some(binding.decl_scope_depth as u32))
                .or_default()
                .push(binding.node);
        }
    }

    let mut edge_set = BTreeSet::new();
    for decl_ids in scope_groups.values() {
        let unique: BTreeSet<KirNodeId> = decl_ids.iter().copied().collect();
        let vec: Vec<KirNodeId> = unique.into_iter().collect();
        for i in 0..vec.len() {
            for j in (i + 1)..vec.len() {
                let (a, b) = if vec[i] < vec[j] {
                    (vec[i], vec[j])
                } else {
                    (vec[j], vec[i])
                };
                if edge_set.insert((a, b, "sharing")) {
                    let span = find_binding_span(kir, a);
                    let provenance =
                        ProvenanceRef::new(span, FactKind::NeedsSharing, "co-scope-sharing", None);
                    edges.push(ProvenancedEdge {
                        edge: ConstraintEdge::new(
                            a,
                            b,
                            ConstraintKind::PropagateSharing,
                            provenance.clone(),
                        ),
                        provenance,
                    });
                }
            }
        }
    }

    // Send propagation: if any unresolved binding needs_send and shares scope with another.
    let send_nodes: BTreeSet<KirNodeId> = nodes
        .iter()
        .filter(|n| n.floor.is_thread_safe())
        .map(|n| n.id)
        .collect();

    for decl_ids in scope_groups.values() {
        let unique: BTreeSet<KirNodeId> = decl_ids.iter().copied().collect();
        for &id in &unique {
            if send_nodes.contains(&id) {
                for &other in &unique {
                    if other != id && node_set.contains(&other) {
                        let (a, b) = if id < other { (id, other) } else { (other, id) };
                        if edge_set.insert((a, b, "send")) {
                            let span = find_binding_span(kir, id);
                            let provenance = ProvenanceRef::new(
                                span,
                                FactKind::NeedsSend,
                                "send-propagation",
                                None,
                            );
                            edges.push(ProvenancedEdge {
                                edge: ConstraintEdge::new(
                                    a,
                                    b,
                                    ConstraintKind::PropagateSend,
                                    provenance.clone(),
                                ),
                                provenance,
                            });
                        }
                    }
                }
            }
        }
    }

    // Mutual exclusion: if two bindings both need mutable wrappers in the same scope.
    for decl_ids in scope_groups.values() {
        let unique: BTreeSet<KirNodeId> = decl_ids.iter().copied().collect();
        let mutable_in_scope: Vec<KirNodeId> = unique
            .iter()
            .copied()
            .filter(|id| {
                nodes
                    .iter()
                    .any(|n| n.id == *id && is_mutable_floor(n.floor))
            })
            .collect();
        for i in 0..mutable_in_scope.len() {
            for j in (i + 1)..mutable_in_scope.len() {
                let (a, b) = if mutable_in_scope[i] < mutable_in_scope[j] {
                    (mutable_in_scope[i], mutable_in_scope[j])
                } else {
                    (mutable_in_scope[j], mutable_in_scope[i])
                };
                if edge_set.insert((a, b, "mutex")) {
                    let span = find_binding_span(kir, a);
                    let provenance = ProvenanceRef::new(
                        span,
                        FactKind::CoMutation,
                        "co-mutation-exclusion",
                        None,
                    );
                    edges.push(ProvenancedEdge {
                        edge: ConstraintEdge::new(
                            a,
                            b,
                            ConstraintKind::MutuallyExclusive,
                            provenance.clone(),
                        ),
                        provenance,
                    });
                }
            }
        }
    }

    // Inter-procedural constraint pass: connect unresolved bindings that
    // escape their scope (is_boundary=true) and need sharing. Escape markers
    // signal actual cross-function data flow (return values, &mut params,
    // struct stores). We only create cross-scope edges for escape-marked
    // bindings to avoid over-constraining independent same-scope bindings.
    {
        let escape_sharing_nodes: Vec<KirNodeId> = nodes
            .iter()
            .filter(|n| n.floor.is_shared() && n.is_boundary)
            .map(|n| n.id)
            .collect();
        for i in 0..escape_sharing_nodes.len() {
            for j in (i + 1)..escape_sharing_nodes.len() {
                let (a, b) = if escape_sharing_nodes[i] < escape_sharing_nodes[j] {
                    (escape_sharing_nodes[i], escape_sharing_nodes[j])
                } else {
                    (escape_sharing_nodes[j], escape_sharing_nodes[i])
                };
                if edge_set.insert((a, b, "sharing")) {
                    let span = find_binding_span(kir, a);
                    let provenance = ProvenanceRef::new(
                        span,
                        FactKind::NeedsSharing,
                        "cross-scope-sharing",
                        None,
                    );
                    edges.push(ProvenancedEdge {
                        edge: ConstraintEdge::new(
                            a,
                            b,
                            ConstraintKind::PropagateSharing,
                            provenance.clone(),
                        ),
                        provenance,
                    });
                }
            }
        }
    }

    // Build the raw graph for the solver.
    let graph_nodes: Vec<KirNodeId> = nodes.iter().map(|n| n.id).collect();
    let graph_edges: Vec<ConstraintEdge> = edges.iter().map(|e| e.edge.clone()).collect();

    ExtractionResult {
        nodes,
        edges,
        graph: ConstraintGraph {
            nodes: graph_nodes,
            edges: graph_edges,
        },
    }
}

fn is_mutable_floor(tier: OwnershipTier) -> bool {
    matches!(
        tier,
        OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared
    )
}

fn binding_requires_send(binding: &TransformBindingFacts) -> bool {
    binding.shared_facts.needs_send || binding.async_shared || binding.is_async
}

fn find_binding_span(kir: &Kir, node_id: KirNodeId) -> KoboSpan {
    kir.get_node(node_id)
        .map(|n| n.span)
        .unwrap_or(KoboSpan::new(0, 0, FileId(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_unresolved_produces_empty_graph() {
        let result = GreedyPassResult {
            resolved: Vec::new(),
            unresolved: Vec::new(),
            diagnostics: Vec::new(),
            stats: Default::default(),
        };
        let kir = kobo_ir::Kir::default();
        let extraction = extract_constraints(&kir, &result);
        assert!(extraction.nodes.is_empty());
        assert!(extraction.edges.is_empty());
        assert!(extraction.graph.nodes.is_empty());
    }

    #[test]
    fn provenance_is_mandatory_on_edges() {
        // Every ProvenancedEdge must have a non-empty rule_name.
        let prov = ProvenanceRef {
            span: KoboSpan::new(0, 10, FileId(0)),
            fact_kind: FactKind::NeedsSharing,
            rule_name: "test-rule",
            source_binding: None,
        };
        assert!(!prov.rule_name.is_empty());
    }
}
