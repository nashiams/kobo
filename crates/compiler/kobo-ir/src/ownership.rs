use crate::kir::BorrowKind;
use crate::node_id::{KirNodeId, KoboAstNodeId};
use crate::resource::ResourceKind;
use crate::span::KoboSpan;

/// Ownership tier assigned to a KIR node. Determines what wrapper type (if any)
/// `kobo-codegen` emits for the corresponding binding in the generated `.rs`
/// file.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
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
    /// `Arc<Mutex<T>>` - mutable shared, thread-safe. Requires
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
    RecursiveType,
    TraitObject,
    StackSizeHeuristic,
}

/// Whether a dead-original assignment can move instead of cloning.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CloneElisionDecision {
    Move,
    Clone,
}

/// Conservative reasons for refusing clone elision.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ElisionFallbackReason {
    MoveSafetyCheckFailed,
}

/// Why a user hint could not be honored by the chosen ownership tier.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum HintConflictReason {
    Aliasing,
    MutableUse,
    SendRequired,
    AsyncDeferred,
    SharedUsage,
    Unknown,
}

/// Typed usage events recorded for a single binding during transform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UseEvent {
    ReadOnly { span: KoboSpan },
    Mutated { span: KoboSpan },
    Moved { span: KoboSpan },
    Escaped { kind: EscapeKind, span: KoboSpan },
    Borrowed { kind: BorrowKind, span: KoboSpan },
}

/// Complete usage history for one binding, sorted by source span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingUsage {
    pub declaration: KoboSpan,
    pub uses: Vec<UseEvent>,
}

/// Derived sharing facts that tier selection consumes instead of raw usage events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedBindingFacts {
    pub node_id: KirNodeId,
    pub mutation_required: bool,
    pub escape_floor: Option<EscapeKind>,
    pub borrow_sites: Vec<KoboSpan>,
    pub read_sites: usize,
    pub mutable_sites: usize,
    pub has_escape: bool,
    pub needs_sharing: bool,
    pub needs_mutable_wrapper: bool,
    pub needs_send: bool,
    pub live_borrow_at_move: bool,
    pub sequential_read_only: bool,
    pub box_reason: Option<BoxReason>,
}

/// All transform facts attached to a single binding before tier choice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformBindingFacts {
    pub node: KirNodeId,
    pub ast_id: KoboAstNodeId,
    pub binding_name: String,
    pub span: KoboSpan,
    pub resource_kind: Option<ResourceKind>,
    pub hint: Option<OwnershipHint>,
    pub hint_span: Option<KoboSpan>,
    pub is_copy_known: bool,
    pub is_generic: bool,
    pub is_async: bool,
    pub usage: BindingUsage,
    pub shared_facts: SharedBindingFacts,
    pub clone_elision: Option<CloneElisionDecision>,
    pub elision_fallback: Option<ElisionFallbackReason>,
}

/// Frozen transform facts consumed by the tier chooser and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformFacts {
    pub bindings: Vec<TransformBindingFacts>,
    pub usages: Vec<BindingUsage>,
    pub shared_facts: Vec<SharedBindingFacts>,
    pub hint_conflicts: Vec<HintConflictFact>,
}

/// Typed payload for warning K0025 when a soft hint is ignored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HintConflictFact {
    pub node: KirNodeId,
    pub hint: OwnershipHint,
    pub hint_span: KoboSpan,
    pub conflict_span: KoboSpan,
    pub chosen_tier: OwnershipTier,
    pub reason: HintConflictReason,
}

/// Chosen ownership tier and explanation for one KIR declaration node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TierDecision {
    pub node: KirNodeId,
    pub tier: OwnershipTier,
    pub reason: TierReason,
    pub annotate: bool,
}

/// Post-choice validation checks that can force a safer tier escalation.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SatisfactionCheck {
    MutationRequiredButTierIsReadOnly,
    EscapeRequiresHeapButTierIsStack,
    SharingRequiredButTierIsExclusive,
    LiveBorrowAtMoveButTierAllowsMove,
    AsyncBoxProhibited,
}

/// Validation result for a chosen tier that failed a safety check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TierViolation {
    pub node_id: KirNodeId,
    pub chosen_tier: OwnershipTier,
    pub failed_check: SatisfactionCheck,
    pub escalated_to: OwnershipTier,
}

/// User-facing reason attached to an ownership annotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TierReason {
    CopyType,
    LocalOnly,
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
}

impl Default for SharedBindingFacts {
    fn default() -> Self {
        Self {
            node_id: KirNodeId(0),
            mutation_required: false,
            escape_floor: None,
            borrow_sites: Vec::new(),
            read_sites: 0,
            mutable_sites: 0,
            has_escape: false,
            needs_sharing: false,
            needs_mutable_wrapper: false,
            needs_send: false,
            live_borrow_at_move: false,
            sequential_read_only: false,
            box_reason: None,
        }
    }
}

impl Default for TransformFacts {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            usages: Vec::new(),
            shared_facts: Vec::new(),
            hint_conflicts: Vec::new(),
        }
    }
}

impl OwnershipTier {
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

    /// Stable label used by inspect output and source maps.
    pub fn label(self) -> &'static str {
        match self {
            OwnershipTier::PlainOwned => "plain",
            OwnershipTier::BoxOwned => "box",
            OwnershipTier::RcShared => "rc",
            OwnershipTier::ArcShared => "arc",
            OwnershipTier::RcMutShared => "rc_refcell",
            OwnershipTier::ArcMutShared => "arc_mutex",
            OwnershipTier::Scoped => "scoped_handle",
            OwnershipTier::Undecided => "plain",
        }
    }

    /// Priority order used by diagnostics when comparing stronger wrapper tiers.
    pub fn priority(self) -> usize {
        match self {
            OwnershipTier::PlainOwned => 1,
            OwnershipTier::RcShared => 2,
            OwnershipTier::ArcShared => 3,
            OwnershipTier::BoxOwned => 4,
            OwnershipTier::RcMutShared => 5,
            OwnershipTier::ArcMutShared => 6,
            OwnershipTier::Scoped => 0,
            OwnershipTier::Undecided => 0,
        }
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

impl CloneElisionDecision {
    /// Returns whether this decision allows moving instead of cloning.
    pub fn is_move(self) -> bool {
        matches!(self, CloneElisionDecision::Move)
    }
}

impl UseEvent {
    /// Source span for this usage event.
    pub fn span(&self) -> KoboSpan {
        match self {
            UseEvent::ReadOnly { span }
            | UseEvent::Mutated { span }
            | UseEvent::Moved { span }
            | UseEvent::Escaped { span, .. }
            | UseEvent::Borrowed { span, .. } => *span,
        }
    }
}

impl BindingUsage {
    /// Creates an empty usage log for a binding declaration span.
    pub fn new(declaration: KoboSpan) -> Self {
        Self {
            declaration,
            uses: Vec::new(),
        }
    }

    /// Appends one typed usage event to the binding log.
    pub fn push_use(&mut self, event: UseEvent) {
        self.uses.push(event);
    }

    /// Sorts usage events by source order for deterministic downstream analysis.
    pub fn sort_uses(&mut self) {
        self.uses.sort_by_key(|event| event.span().start);
    }
}

impl TransformFacts {
    /// Builds a fact bundle from fully-populated binding facts.
    pub fn from_bindings(
        bindings: Vec<TransformBindingFacts>,
        hint_conflicts: Vec<HintConflictFact>,
    ) -> Self {
        let usages = bindings
            .iter()
            .map(|binding| binding.usage.clone())
            .collect();
        let shared_facts = bindings
            .iter()
            .map(|binding| binding.shared_facts.clone())
            .collect();

        Self {
            bindings,
            usages,
            shared_facts,
            hint_conflicts,
        }
    }

    /// Rebuilds the flat usage and shared-fact views from the binding records.
    pub fn sync_views(&mut self) {
        self.usages = self
            .bindings
            .iter()
            .map(|binding| binding.usage.clone())
            .collect();
        self.shared_facts = self
            .bindings
            .iter()
            .map(|binding| binding.shared_facts.clone())
            .collect();
    }

    /// Returns the facts for one declaration node.
    pub fn binding(&self, node: KirNodeId) -> Option<&TransformBindingFacts> {
        self.bindings.iter().find(|binding| binding.node == node)
    }

    /// Returns the mutable facts for one declaration node.
    pub fn binding_mut(&mut self, node: KirNodeId) -> Option<&mut TransformBindingFacts> {
        self.bindings
            .iter_mut()
            .find(|binding| binding.node == node)
    }

    /// Iterates over all binding facts in deterministic insertion order.
    pub fn iter_bindings(&self) -> impl Iterator<Item = &TransformBindingFacts> {
        self.bindings.iter()
    }
}

/// Derives sharing facts from one binding's usage log plus any clone-elision decision.
pub fn derive_shared_facts(
    usage: &BindingUsage,
    elision: Option<&CloneElisionDecision>,
) -> SharedBindingFacts {
    let mut shared_facts = SharedBindingFacts::default();

    let read_sites = usage
        .uses
        .iter()
        .filter(|event| matches!(event, UseEvent::ReadOnly { .. }))
        .count();
    let borrow_sites = usage
        .uses
        .iter()
        .filter_map(|event| match event {
            UseEvent::Borrowed { span, .. } => Some(*span),
            _ => None,
        })
        .collect::<Vec<_>>();
    let immutable_borrow_sites = usage
        .uses
        .iter()
        .filter_map(|event| match event {
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span,
            } => Some(*span),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mutable_sites = usage
        .uses
        .iter()
        .filter(|event| {
            matches!(
                event,
                UseEvent::Mutated { .. }
                    | UseEvent::Borrowed {
                        kind: BorrowKind::Mutable,
                        ..
                    }
            )
        })
        .count();
    let escape_floor = usage
        .uses
        .iter()
        .filter_map(|event| match event {
            UseEvent::Escaped { kind, .. } => Some(*kind),
            _ => None,
        })
        .max_by_key(|kind| kind.severity());
    let first_move = usage.uses.iter().find_map(|event| match event {
        UseEvent::Moved { span } => Some(*span),
        _ => None,
    });
    let confirmed_move = elision.is_some_and(|decision| decision.is_move());
    let has_borrow_before_move = first_move.is_some_and(|move_span| {
        borrow_sites
            .iter()
            .any(|borrow_site| borrow_site.start <= move_span.start)
    });
    let has_later_use_after_move = first_move.is_some_and(|move_span| {
        usage
            .uses
            .iter()
            .filter(|event| !matches!(event, UseEvent::Moved { .. }))
            .any(|event| event.span().start > move_span.start)
    });
    let live_borrow_at_move = has_borrow_before_move && has_later_use_after_move;
    let borrow_only_reach = borrow_sites.len() == 1 && read_sites == 0 && mutable_sites == 0;

    shared_facts.read_sites = read_sites;
    shared_facts.mutable_sites = mutable_sites;
    shared_facts.mutation_required = mutable_sites > 0;
    shared_facts.escape_floor = escape_floor;
    shared_facts.borrow_sites = borrow_sites;
    shared_facts.has_escape = shared_facts.escape_floor.is_some();
    shared_facts.needs_mutable_wrapper = shared_facts.mutation_required;
    shared_facts.live_borrow_at_move = live_borrow_at_move;
    let sequential_read_sites = read_sites >= 2 && immutable_borrow_sites.is_empty();
    let sequential_borrow_sites = immutable_borrow_sites.len() >= 2 && read_sites == 0;
    shared_facts.sequential_read_only = mutable_sites == 0
        && !live_borrow_at_move
        && (sequential_read_sites || sequential_borrow_sites);
    shared_facts.needs_sharing = if confirmed_move {
        false
    } else if live_borrow_at_move {
        true
    } else if shared_facts.borrow_sites.len() >= 2 {
        true
    } else if shared_facts.read_sites >= 2 {
        true
    } else {
        !borrow_only_reach && false
    };

    shared_facts
}

/// Builds the flat transform fact bundle from usage, elision, and hint-conflict vectors.
pub fn derive_transform_facts(
    usages: Vec<BindingUsage>,
    elisions: Vec<CloneElisionDecision>,
    hints: Vec<HintConflictFact>,
) -> TransformFacts {
    let shared_facts = usages
        .iter()
        .enumerate()
        .map(|(index, usage)| {
            let elision = elisions.get(index);
            derive_shared_facts(usage, elision)
        })
        .collect();

    TransformFacts {
        bindings: Vec::new(),
        usages,
        shared_facts,
        hint_conflicts: hints,
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
        }
    }
}

impl TierReason {
    /// Renders the inspect annotation wording for this tier reason.
    pub fn annotation_text(&self, _tier: OwnershipTier) -> String {
        match self {
            TierReason::CopyType => "copy type".to_owned(),
            TierReason::LocalOnly => "local-only non-Copy binding".to_owned(),
            TierReason::DeadOriginalAfterAssignment => "dead original after assignment".to_owned(),
            TierReason::CloneElisionFallback(ElisionFallbackReason::MoveSafetyCheckFailed) => {
                "clone-elision fallback: move safety check failed - conservative clone".to_owned()
            }
            TierReason::HeapStable(box_reason) => match box_reason {
                BoxReason::RecursiveType => {
                    "single owner, heap required (recursive type)".to_owned()
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
            TierReason::ValidationEscalation(check) => {
                format!("validation escalation: {}", check.description())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{derive_shared_facts, BindingUsage, BorrowKind, CloneElisionDecision, UseEvent};
    use crate::node_id::FileId;
    use crate::span::KoboSpan;

    fn span(start: u32) -> KoboSpan {
        KoboSpan::new(start, start + 1, FileId(0))
    }

    fn usage(events: Vec<UseEvent>) -> BindingUsage {
        BindingUsage {
            declaration: span(0),
            uses: events,
        }
    }

    #[test]
    fn single_read_only_use_stays_local() {
        let shared_facts =
            derive_shared_facts(&usage(vec![UseEvent::ReadOnly { span: span(10) }]), None);

        assert!(!shared_facts.needs_sharing);
        assert!(!shared_facts.sequential_read_only);
    }

    #[test]
    fn repeated_ephemeral_borrows_are_marked_as_sequential_read_only() {
        let shared_facts = derive_shared_facts(
            &usage(vec![
                UseEvent::Borrowed {
                    kind: BorrowKind::Immutable,
                    span: span(10),
                },
                UseEvent::Borrowed {
                    kind: BorrowKind::Immutable,
                    span: span(20),
                },
            ]),
            None,
        );

        assert!(shared_facts.needs_sharing);
        assert!(shared_facts.sequential_read_only);
    }

    #[test]
    fn dead_borrow_before_move_does_not_force_sharing() {
        let shared_facts = derive_shared_facts(
            &usage(vec![
                UseEvent::Borrowed {
                    kind: BorrowKind::Immutable,
                    span: span(10),
                },
                UseEvent::Moved { span: span(20) },
            ]),
            Some(&CloneElisionDecision::Move),
        );

        assert!(!shared_facts.needs_sharing);
        assert!(!shared_facts.live_borrow_at_move);
    }

    #[test]
    fn live_borrow_at_move_requires_sharing() {
        let shared_facts = derive_shared_facts(
            &usage(vec![
                UseEvent::Borrowed {
                    kind: BorrowKind::Immutable,
                    span: span(10),
                },
                UseEvent::Moved { span: span(20) },
                UseEvent::ReadOnly { span: span(30) },
            ]),
            None,
        );

        assert!(shared_facts.needs_sharing);
        assert!(shared_facts.live_borrow_at_move);
    }

    #[test]
    fn mutable_borrow_sets_mutation_required() {
        let shared_facts = derive_shared_facts(
            &usage(vec![UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                span: span(10),
            }]),
            None,
        );

        assert!(shared_facts.mutation_required);
        assert!(shared_facts.needs_mutable_wrapper);
    }

    #[test]
    fn shared_binding_facts_are_a_pure_function_of_usage() {
        let usage = usage(vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10),
            },
            UseEvent::Moved { span: span(20) },
            UseEvent::ReadOnly { span: span(30) },
        ]);

        let first = derive_shared_facts(&usage, None);
        let second = derive_shared_facts(&usage, None);

        assert_eq!(first, second);
    }
}
