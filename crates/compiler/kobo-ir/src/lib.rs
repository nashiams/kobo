pub mod debt;
mod kir;
mod mode;
mod node_id;
mod ownership;
mod resource;
mod solution_map;
mod span;
pub mod strict;
#[cfg(test)]
mod strict_tests;

pub use debt::{
    AcknowledgedDebtRecord, ComplexityBreakdown, DebtComplexityTier, DebtReport, DebtSiteRecord,
    FieldTypeShape, KirStructDef, KirStructFieldDef, WarnEarlyFact, WarnEarlyPattern,
    WrapperInventory,
};
pub use kir::{BorrowKind, NodeKind, UseKind};
pub use kir::{Kir, KirNode, RelaxAttrError};
pub use node_id::{
    CfgBlockId, FileEntry, FileId, FileSet, FileSetBuilder, KirNodeId, KoboAstNodeId, NodeIdGen,
};
pub use ownership::{
    derive_shared_facts, derive_transform_facts, BindingUsage, BoxReason, CloneElisionCandidate,
    CloneElisionDecision, ElisionFallbackReason, ElisionSkipReason, EscapeKind, HintConflictFact,
    HintConflictReason, OwnershipHint, OwnershipTier, SatisfactionCheck, SharedBindingFacts,
    TierDecision, TierReason, TierViolation, TransformBindingFacts, TransformFacts, UseEvent,
};
pub use resource::ResourceKind;
pub use solution_map::SolutionMap;
pub use span::KoboSpan;
pub use mode::KoboMode;
pub use strict::{
    CaptureAccessKind, CaptureSet, CapturedBinding, ClosureCaptureDetail, ClosureCaptureMode,
    NestedStrictBlock, StrictBoundaryFact, StrictBoundaryViolation, StrictFnMode,
};
