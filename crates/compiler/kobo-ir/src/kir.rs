use std::collections::HashMap;

use crate::node_id::{CfgBlockId, KirNodeId, KoboAstNodeId};
use crate::ownership::{OwnershipTier, TierDecision, TransformFacts};
use crate::resource::ResourceKind;
use crate::span::KoboSpan;

// --- Types first ---

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum UseKind {
    Read,
    Write,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BorrowKind {
    Immutable,
    Mutable,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum NodeKind {
    ScopeStart,
    ScopeEnd,
    Decl,
    Use(UseKind),
    Move,
    Borrow(BorrowKind),
}

/// A single node in the Kobo Intermediate Representation.
///
/// KIR nodes are produced by `kobo-transform` and frozen thereafter.
/// No subsequent pass mutates a `KirNode`. The `kobo-migrate` solver
/// writes to a `SolutionMap` instead.
#[derive(Clone, Debug)]
pub struct KirNode {
    pub id: KirNodeId,
    pub kind: NodeKind,
    /// Back-pointer to the originating Kobo AST node.
    pub ast_id: Option<KoboAstNodeId>,
    /// Ownership tier assigned during transform. `Undecided` until migration runs.
    pub ownership: OwnershipTier,
    /// Set for resource-kind bindings (files, locks, sockets). `None` otherwise.
    pub resource_kind: Option<ResourceKind>,
    /// CFG block this node belongs to. `None` until the CFG pass runs (v0.7).
    pub cfg_block: Option<CfgBlockId>,
    /// Source location in the `.kobo` file.
    pub span: KoboSpan,
    /// For use, move, and borrow nodes: points back to the declaration site.
    pub decl_id: Option<KirNodeId>,
}

/// The Kobo Intermediate Representation for a single source file.
///
/// Frozen after `kobo-transform` completes.
#[derive(Debug, Default)]
pub struct Kir {
    nodes: Vec<KirNode>,
    /// Maps from AST node ID back to KIR node ID for fast lookup.
    ast_to_kir: HashMap<KoboAstNodeId, KirNodeId>,
    transform_facts: TransformFacts,
    tier_decisions: Vec<TierDecision>,
}

// --- Public read API ---

impl Kir {
    pub fn from_nodes(nodes: Vec<KirNode>) -> Self {
        let ast_to_kir = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Decl)
            .filter_map(|node| node.ast_id.map(|ast_id| (ast_id, node.id)))
            .collect();

        Self {
            nodes,
            ast_to_kir,
            transform_facts: TransformFacts::default(),
            tier_decisions: Vec::new(),
        }
    }

    pub fn get_node(&self, id: KirNodeId) -> Option<&KirNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = &KirNode> {
        self.nodes.iter()
    }

    pub fn iter_nodes_mut(&mut self) -> impl Iterator<Item = &mut KirNode> {
        self.nodes.iter_mut()
    }

    pub fn nodes_in_source_order(&self) -> impl Iterator<Item = &KirNode> {
        self.nodes.iter()
    }

    pub fn iter_decl_nodes(&self) -> impl Iterator<Item = &KirNode> {
        self.nodes.iter().filter(|node| node.kind == NodeKind::Decl)
    }

    pub fn kir_for_ast(&self, ast_id: KoboAstNodeId) -> Option<KirNodeId> {
        self.ast_to_kir.get(&ast_id).copied()
    }

    pub fn transform_facts(&self) -> &TransformFacts
    {
        &self.transform_facts
    }

    pub fn set_transform_facts(&mut self, facts: TransformFacts) {
        self.transform_facts = facts;
    }

    pub fn tier_decision(&self, node_id: KirNodeId) -> Option<&TierDecision> {
        self.tier_decisions
            .iter()
            .find(|decision| decision.node == node_id)
    }

    pub fn iter_tier_decisions(&self) -> impl Iterator<Item = &TierDecision> {
        self.tier_decisions.iter()
    }

    pub fn set_tier_decisions(&mut self, decisions: Vec<TierDecision>) {
        self.tier_decisions = decisions;
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
