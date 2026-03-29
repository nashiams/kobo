mod decision;
mod elision;
mod facts;
#[cfg(test)]
mod tests;

pub use decision::{
    BoxReason, EscapeKind, OwnershipHint, OwnershipTier, SatisfactionCheck, TierDecision,
    TierReason, TierViolation,
};
pub use elision::{
    CloneElisionCandidate, CloneElisionDecision, ElisionFallbackReason, ElisionSkipReason,
};
pub use facts::{
    derive_shared_facts, derive_transform_facts, BindingUsage, HintConflictFact,
    HintConflictReason, SharedBindingFacts, TransformBindingFacts, TransformFacts, UseEvent,
};
