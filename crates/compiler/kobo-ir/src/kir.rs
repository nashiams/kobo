use std::collections::HashMap;

use crate::node_id::{CfgBlockId, KirNodeId, KoboAstNodeId};
use crate::ownership::OwnershipTier;
use crate::resource::ResourceKind;
use crate::span::KoboSpan;

// --- Types first ---

/// A single node in the Kobo Intermediate Representation.
///
/// KIR nodes are produced by `kobo-transform` and frozen thereafter.
/// No subsequent pass mutates a `KirNode`. The `kobo-migrate` solver
/// writes to a `SolutionMap` instead.
#[derive(Clone, Debug)]
pub struct KirNode {
    pub id: KirNodeId,
    /// Back-pointer to the originating Kobo AST node.
    pub ast_id: KoboAstNodeId,
    /// Ownership tier assigned during transform. `Undecided` until migration runs.
    pub ownership: OwnershipTier,
    /// Set for resource-kind bindings (files, locks, sockets). `None` otherwise.
    pub resource_kind: Option<ResourceKind>,
    /// CFG block this node belongs to. `None` until the CFG pass runs (v0.7).
    pub cfg_block: Option<CfgBlockId>,
    /// Source location in the `.kobo` file.
    pub span: KoboSpan,
}

/// The Kobo Intermediate Representation for a single source file.
///
/// Frozen after `kobo-transform` completes.
#[derive(Debug, Default)]
pub struct Kir {
    nodes: Vec<KirNode>,
    /// Maps from AST node ID back to KIR node ID for fast lookup.
    ast_to_kir: HashMap<KoboAstNodeId, KirNodeId>,
}

// --- Public read API ---

impl Kir {
    pub fn from_nodes(nodes: Vec<KirNode>) -> Self {
        let ast_to_kir = nodes.iter().map(|node| (node.ast_id, node.id)).collect();

        Self { nodes, ast_to_kir }
    }

    pub fn get_node(&self, id: KirNodeId) -> Option<&KirNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = &KirNode> {
        self.nodes.iter()
    }

    pub fn kir_for_ast(&self, ast_id: KoboAstNodeId) -> Option<KirNodeId> {
        self.ast_to_kir.get(&ast_id).copied()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
