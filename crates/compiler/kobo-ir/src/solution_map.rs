use std::collections::HashMap;

use crate::node_id::KirNodeId;
use crate::ownership::OwnershipTier;

/// Resolved ownership assignment for every previously-`Undecided` KIR node.
///
/// Produced by `kobo-migrate`. `kobo-codegen` merges this with KIR to
/// determine the final wrapper type for each binding.
///
/// KIR nodes that already have a decided tier are not present here —
/// `kobo-codegen` falls back to `KirNode.ownership` for those.
pub struct SolutionMap {
    inner: HashMap<KirNodeId, OwnershipTier>,
}

impl SolutionMap {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Records a resolved tier for a previously-`Undecided` node.
    pub fn insert(&mut self, id: KirNodeId, tier: OwnershipTier) {
        self.inner.insert(id, tier);
    }

    /// Returns the resolved tier for a node, if present.
    pub fn get(&self, id: KirNodeId) -> Option<OwnershipTier> {
        self.inner.get(&id).copied()
    }

    pub fn contains(&self, id: KirNodeId) -> bool {
        self.inner.contains_key(&id)
    }

    /// Returns the resolved tier for `id`, or falls back to `fallback` if absent.
    pub fn resolve(&self, id: KirNodeId, fallback: OwnershipTier) -> OwnershipTier {
        self.inner.get(&id).copied().unwrap_or(fallback)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Default for SolutionMap {
    fn default() -> Self {
        Self::new()
    }
}
