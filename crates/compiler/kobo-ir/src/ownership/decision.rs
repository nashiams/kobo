use super::elision::ElisionFallbackReason;

/// Ownership tier assigned to a KIR node. Determines what wrapper type (if any)
/// `kobo-codegen` emits for the corresponding binding in the generated `.rs`
/// file.
#[derive(
    Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum OwnershipTier {
    /// `T` - moved. No wrapper, no allocation. Zero overhead.
    PlainOwned,
    /// `Box<T>` - single owner, heap-allocated.
    BoxOwned,
    /// `Rc<T>` - read-only shared reference count. No mutation.
    RcShared,
    /// `Arc<T>` - read-only shared, thread-safe. No mutation.
    ArcShared,
    /// `Rc<RefCell<T>>` - mutable shared, single-threaded. Script-mode default.
    RcMutShared,
    /// `Arc<RwLock<T>>` - mutable shared, thread-safe. Requires
    /// `#[kobo::async_shared]`.
    ArcMutShared,
    /// `ScopedHandle<T>` - resource kind with enforced single ownership.
    Scoped,
    /// Not yet resolved. `kobo-migrate` produces a `SolutionMap` that resolves
    /// this.
    Undecided,
}

/// User-authored soft ownership preference attached to a binding.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum OwnershipHint {
    Move,
    Exclusive,
    Shared,
    Async,
}

/// Ways a binding can leave local ownership and require a wrapper floor.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum EscapeKind {
    ReturnedFromFunction,
    StoredInStruct,
    PassedToOpaqueCall,
    CapturedByEscapingClosure,
}

/// Heap-stability reasons that justify `Box<T>` over stack ownership.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BoxReason {
    /// Reserved for future type-definition validation work. v0.3 transform does
    /// not emit this from binding-level lowering.
    RecursiveType,
    TraitObject,
    StackSizeHeuristic,
}

/// Post-choice validation checks that can force a safer tier escalation.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SatisfactionCheck {
    MutationRequiredButTierIsReadOnly,
    EscapeRequiresHeapButTierIsStack,
    SharingRequiredButTierIsExclusive,
    LiveBorrowAtMoveButTierAllowsMove,
    AsyncBoxProhibited,
    /// `ReturnedFromFunction` escape: producing `Box<T>` requires rewriting the
    /// function's return type and all call sites - deferred to v0.4.
    /// Escalated to `RcShared` in v0.3; the annotation makes the gap visible.
    ReturnEscapeBoxDeferred,
}

/// Validation result for a chosen tier that failed a safety check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TierViolation {
    pub node_id: crate::node_id::KirNodeId,
    pub chosen_tier: OwnershipTier,
    pub failed_check: SatisfactionCheck,
    pub escalated_to: OwnershipTier,
}

/// User-facing reason attached to an ownership annotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TierReason {
    CopyType,
    LocalOnly,
    MoveRebind,
    DeadOriginalAfterAssignment,
    CloneElisionFallback(ElisionFallbackReason),
    HeapStable(BoxReason),
    ReadOnlyShared {
        read_sites: usize,
        sequential_only: bool,
    },
    SendRequiredShared,
    MutableSharedLastResort,
    GenericWrapperFloor,
    AsyncBoxDeferred,
    ResourceWrapper,
    ValidationEscalation(SatisfactionCheck),
    /// `#[kobo::async_shared]` opt-in attribute forced an Arc tier [BUG 7].
    AsyncSharedAttribute,
}

/// Chosen ownership tier and explanation for one KIR declaration node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TierDecision {
    pub node: crate::node_id::KirNodeId,
    pub tier: OwnershipTier,
    pub reason: TierReason,
    pub annotate: bool,
}

impl OwnershipTier {
    pub const SOLVED_LATTICE: [OwnershipTier; 7] = [
        OwnershipTier::PlainOwned,
        OwnershipTier::BoxOwned,
        OwnershipTier::RcShared,
        OwnershipTier::ArcShared,
        OwnershipTier::RcMutShared,
        OwnershipTier::ArcMutShared,
        OwnershipTier::Scoped,
    ];

    /// Returns whether this tier uses shared ownership semantics.
    pub fn is_shared(&self) -> bool {
        matches!(
            self,
            OwnershipTier::RcShared
                | OwnershipTier::ArcShared
                | OwnershipTier::RcMutShared
                | OwnershipTier::ArcMutShared
        )
    }

    /// Returns whether this tier uses thread-safe shared ownership.
    pub fn is_thread_safe(&self) -> bool {
        matches!(self, OwnershipTier::ArcShared | OwnershipTier::ArcMutShared)
    }

    /// Returns whether moves of this tier must lower as wrapper reads instead.
    pub fn is_cloneable_wrapper(&self) -> bool {
        matches!(
            self,
            OwnershipTier::RcShared
                | OwnershipTier::ArcShared
                | OwnershipTier::RcMutShared
                | OwnershipTier::ArcMutShared
        )
    }

    /// Returns whether migration has already resolved this tier.
    pub fn is_decided(&self) -> bool {
        !matches!(self, OwnershipTier::Undecided)
    }

    pub fn is_solved_lattice_member(self) -> bool {
        !matches!(self, OwnershipTier::Undecided)
    }

    pub fn lattice_join(self, other: OwnershipTier) -> Option<OwnershipTier> {
        if !self.is_solved_lattice_member() || !other.is_solved_lattice_member() {
            return None;
        }
        if self.priority() >= other.priority() {
            Some(self)
        } else {
            Some(other)
        }
    }

    pub fn lattice_leq(self, other: OwnershipTier) -> Option<bool> {
        if !self.is_solved_lattice_member() || !other.is_solved_lattice_member() {
            return None;
        }
        Some(self.priority() <= other.priority())
    }

    /// Stable label used by inspect output and source maps.
    pub fn label(self) -> &'static str {
        match self {
            OwnershipTier::PlainOwned => "plain",
            OwnershipTier::BoxOwned => "box",
            OwnershipTier::RcShared => "rc",
            OwnershipTier::ArcShared => "arc",
            OwnershipTier::RcMutShared => "rc_refcell",
            OwnershipTier::ArcMutShared => "arc_rwlock",
            OwnershipTier::Scoped => "scoped_handle",
            OwnershipTier::Undecided => "plain",
        }
    }

    /// Relative floor strength. Canonical ordering shared with `tier_rank()`.
    ///
    /// Undecided(0) < PlainOwned(1) < BoxOwned(2) < RcShared(3) < ArcShared(4)
    /// < RcMutShared(5) < ArcMutShared(6) < Scoped(7)
    pub fn priority(self) -> usize {
        match self {
            OwnershipTier::Undecided => 0,
            OwnershipTier::PlainOwned => 1,
            OwnershipTier::BoxOwned => 2,
            OwnershipTier::RcShared => 3,
            OwnershipTier::ArcShared => 4,
            OwnershipTier::RcMutShared => 5,
            OwnershipTier::ArcMutShared => 6,
            OwnershipTier::Scoped => 7,
        }
    }

    /// Greedy ladder position. Delegates to canonical ordering.
    pub fn greedy_priority(self) -> usize {
        self.priority()
    }
}

impl OwnershipHint {
    /// Stable hint spelling used by diagnostics and snapshots.
    pub fn as_str(self) -> &'static str {
        match self {
            OwnershipHint::Move => "move",
            OwnershipHint::Exclusive => "exclusive",
            OwnershipHint::Shared => "shared",
            OwnershipHint::Async => "async",
        }
    }
}

impl EscapeKind {
    /// Relative floor strength for escape kinds when multiple escapes are present.
    pub fn severity(self) -> usize {
        match self {
            EscapeKind::ReturnedFromFunction => 3,
            EscapeKind::StoredInStruct => 2,
            EscapeKind::PassedToOpaqueCall => 1,
            EscapeKind::CapturedByEscapingClosure => 4,
        }
    }
}

impl TierDecision {
    /// Renders the user-facing annotation text for this chosen tier.
    pub fn annotation_text(&self) -> String {
        self.reason.annotation_text(self.tier)
    }
}

impl SatisfactionCheck {
    /// Human-readable description of the failed validation check.
    pub fn description(self) -> &'static str {
        match self {
            SatisfactionCheck::MutationRequiredButTierIsReadOnly => {
                "mutable use required a shared-mutable tier"
            }
            SatisfactionCheck::EscapeRequiresHeapButTierIsStack => {
                "heap-stable escape required heap placement"
            }
            SatisfactionCheck::SharingRequiredButTierIsExclusive => {
                "shared use required a shared tier"
            }
            SatisfactionCheck::LiveBorrowAtMoveButTierAllowsMove => {
                "live borrow at move required sharing"
            }
            SatisfactionCheck::AsyncBoxProhibited => "Box<T> is deferred inside async fn in v0.3",
            SatisfactionCheck::ReturnEscapeBoxDeferred => {
                "Box<T> requires return-type rewrite (v0.4); escalated to Rc<T>"
            }
        }
    }
}

impl TierReason {
    /// Renders the inspect annotation wording for this tier reason.
    pub fn annotation_text(&self, _tier: OwnershipTier) -> String {
        match self {
            TierReason::CopyType => "copy type".to_owned(),
            TierReason::LocalOnly => "local-only non-Copy binding".to_owned(),
            TierReason::MoveRebind => "moved then dead (freeze-and-rotate)".to_owned(),
            TierReason::DeadOriginalAfterAssignment => "dead original after assignment".to_owned(),
            TierReason::CloneElisionFallback(ElisionFallbackReason::MoveSafetyCheckFailed) => {
                "clone-elision fallback: move safety check failed - conservative clone".to_owned()
            }
            TierReason::HeapStable(box_reason) => match box_reason {
                BoxReason::RecursiveType => {
                    "single owner, heap required (recursive type; future validation path)"
                        .to_owned()
                }
                BoxReason::TraitObject => "single owner, heap required (trait object)".to_owned(),
                BoxReason::StackSizeHeuristic => {
                    "single owner, heap required (size heuristic)".to_owned()
                }
            },
            TierReason::ReadOnlyShared {
                read_sites,
                sequential_only,
            } if *sequential_only => {
                format!("sequential read-only; &T likely at migration ({read_sites} call sites)")
            }
            TierReason::ReadOnlyShared {
                read_sites,
                sequential_only: _,
            } => format!("read-only shared across {read_sites} call sites"),
            TierReason::SendRequiredShared => "Send-required shared".to_owned(),
            TierReason::MutableSharedLastResort => "mutable shared, last resort".to_owned(),
            TierReason::GenericWrapperFloor => "generic T: Copy unknown".to_owned(),
            TierReason::AsyncBoxDeferred => "Box<T> deferred: async fn (v0.7)".to_owned(),
            TierReason::ResourceWrapper => "resource wrapper".to_owned(),
            TierReason::AsyncSharedAttribute => "explicit async-shared opt-in: Arc tier".to_owned(),
            TierReason::ValidationEscalation(SatisfactionCheck::ReturnEscapeBoxDeferred) => {
                "return escape: Box<T> requires signature rewrite (v0.4)".to_owned()
            }
            TierReason::ValidationEscalation(check) => {
                format!("validation escalation: {}", check.description())
            }
        }
    }
}
