mod kir;
mod node_id;
mod ownership;
mod resource;
mod solution_map;
mod span;

pub use kir::{BorrowKind, NodeKind, UseKind};
pub use kir::{Kir, KirNode};
pub use node_id::{
    CfgBlockId, FileEntry, FileId, FileSet, FileSetBuilder, KirNodeId, KoboAstNodeId, NodeIdGen,
};
pub use ownership::{
    derive_shared_facts, derive_transform_facts, BindingUsage, BoxReason, CloneElisionDecision,
    ElisionFallbackReason, EscapeKind, HintConflictFact, HintConflictReason, OwnershipHint,
    OwnershipTier, SatisfactionCheck, SharedBindingFacts, TierDecision, TierReason, TierViolation,
    TransformBindingFacts, TransformFacts, UseEvent,
};
pub use resource::ResourceKind;
pub use solution_map::SolutionMap;
pub use span::KoboSpan;
