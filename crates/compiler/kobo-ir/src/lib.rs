pub mod core;
pub mod debt;
mod guarantee_policy;
mod kir;
mod mode;
mod node_id;
mod ownership;
mod resource;
pub mod scenario;
mod solution_map;
mod span;
pub mod strict;
#[cfg(test)]
mod strict_tests;

pub use core::{
    lower_program as lower_core_program, CoreBlock, CoreFunction, CoreProgram, CoreStatement,
    CoreStatementKind, CoreTerminator, CoreTerminatorKind,
};
pub use debt::{
    AcknowledgedDebtRecord, ComplexityBreakdown, DebtComplexityTier, DebtReport, DebtSiteRecord,
    FieldTypeShape, KirStructDef, KirStructFieldDef, OwnershipDebtCode, OwnershipDebtKind,
    OwnershipDebtRecord, OwnershipDebtSeverity, WarnEarlyFact, WarnEarlyPattern, WrapperInventory,
};
pub use guarantee_policy::{
    ErrorPolicy, GuaranteeDimension, GuaranteeDowngrade, GuaranteeLevel, GuaranteePolicy,
    GuaranteeProfile, GuaranteeSet,
};
pub use kir::{BorrowKind, NodeKind, UseKind};
pub use kir::{
    FieldCapabilityField, FieldCapabilityView, Kir, KirNode, MigrateSite, MigrateTarget,
    MustCallAction, MustCallAttrError, MustCallObligation, RelaxAttrError,
};
pub use mode::LegacyMode;
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
pub use scenario::{
    ProtocolTemplateDefinition, ProtocolTemplateRegistry, ScenarioBoundary,
    ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioCallGraphScc,
    ScenarioCoreCfgBlock, ScenarioCoreCfgEdge, ScenarioCoreCfgFacts, ScenarioCoreTerminatorKind,
    ScenarioCoverageFacts, ScenarioExternalCallShape, ScenarioLifecycleTemplate,
    ScenarioLifecycleTemplateSource, ScenarioModeledBoundary, ScenarioOp, ScenarioOpKind,
    ScenarioProgram,
};
pub use solution_map::SolutionMap;
pub use span::KoboSpan;
pub use strict::{
    AsyncViolationFact, AsyncViolationKind, CaptureAccessKind, CaptureSet, CapturedBinding,
    ClosureCaptureDetail, ClosureCaptureMode, NestedStrictBlock, StrictBoundaryFact,
    StrictBoundaryViolation, StrictFnMode,
};
