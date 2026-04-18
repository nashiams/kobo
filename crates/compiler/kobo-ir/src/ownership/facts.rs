use crate::kir::BorrowKind;
use crate::node_id::{KirNodeId, KoboAstNodeId};
use crate::resource::ResourceKind;
use crate::span::KoboSpan;

use super::decision::{BoxReason, EscapeKind, OwnershipHint, OwnershipTier};
use super::elision::{CloneElisionDecision, ElisionFallbackReason, ElisionSkipReason};

/// Typed usage events recorded for a single binding during transform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UseEvent {
    ReadOnly { span: KoboSpan },
    Mutated { span: KoboSpan },
    Moved { span: KoboSpan, scope_depth: usize },
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
    /// `#[kobo::async_shared]` opt-in — forces Arc tier (BUG 7).
    pub async_shared: bool,
    pub usage: BindingUsage,
    pub shared_facts: SharedBindingFacts,
    pub clone_elision: Option<CloneElisionDecision>,
    pub elision_fallback: Option<ElisionFallbackReason>,
    pub plain_clone_alias: bool,
    pub plain_clone_source: Option<KirNodeId>,
    pub plain_clone_move_span: Option<KoboSpan>,
    pub elision_skip_reason: Option<ElisionSkipReason>,
    /// Scope depth at which this binding was declared (0 = function body).
    pub decl_scope_depth: usize,
    /// Read spans from reference-returning methods (not eligible for extraction).
    pub ref_returning_read_spans: Vec<KoboSpan>,
}

/// Frozen transform facts consumed by the tier chooser and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformFacts {
    pub bindings: Vec<TransformBindingFacts>,
    pub usages: Vec<BindingUsage>,
    pub shared_facts: Vec<SharedBindingFacts>,
    pub hint_conflicts: Vec<HintConflictFact>,
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

impl UseEvent {
    /// Source span for this usage event.
    pub fn span(&self) -> KoboSpan {
        match self {
            UseEvent::ReadOnly { span }
            | UseEvent::Mutated { span }
            | UseEvent::Moved { span, .. }
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
    let read_sites = count_read_sites(usage);
    let borrow_sites = collect_borrow_sites(usage);
    let immutable_borrow_sites = collect_immutable_borrow_sites(usage);
    let mutable_sites = count_mutable_sites(usage);
    let mutation_required = mutable_sites > 0;
    let escape_floor = highest_escape_floor(usage);
    let has_escape = escape_floor.is_some();
    let live_borrow_at_move = has_live_borrow_at_move(usage, &borrow_sites, elision);
    let sequential_read_only = is_sequential_read_only(
        read_sites,
        mutable_sites,
        live_borrow_at_move,
        &immutable_borrow_sites,
    );
    let needs_sharing = needs_sharing(read_sites, borrow_sites.len(), live_borrow_at_move, elision);

    SharedBindingFacts {
        read_sites,
        mutable_sites,
        mutation_required,
        escape_floor,
        borrow_sites,
        has_escape,
        needs_sharing,
        needs_mutable_wrapper: mutation_required,
        live_borrow_at_move,
        sequential_read_only,
        ..Default::default()
    }
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
        .map(|(index, usage)| derive_shared_facts(usage, elisions.get(index)))
        .collect();

    TransformFacts {
        bindings: Vec::new(),
        usages,
        shared_facts,
        hint_conflicts: hints,
    }
}

fn count_read_sites(usage: &BindingUsage) -> usize {
    usage
        .uses
        .iter()
        .filter(|event| matches!(event, UseEvent::ReadOnly { .. }))
        .count()
}

fn collect_borrow_sites(usage: &BindingUsage) -> Vec<KoboSpan> {
    usage
        .uses
        .iter()
        .filter_map(|event| match event {
            UseEvent::Borrowed { span, .. } => Some(*span),
            _ => None,
        })
        .collect()
}

fn collect_immutable_borrow_sites(usage: &BindingUsage) -> Vec<KoboSpan> {
    usage
        .uses
        .iter()
        .filter_map(|event| match event {
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span,
            } => Some(*span),
            _ => None,
        })
        .collect()
}

fn count_mutable_sites(usage: &BindingUsage) -> usize {
    usage
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
        .count()
}

fn highest_escape_floor(usage: &BindingUsage) -> Option<EscapeKind> {
    usage
        .uses
        .iter()
        .filter_map(|event| match event {
            UseEvent::Escaped { kind, .. } => Some(*kind),
            _ => None,
        })
        .max_by_key(|kind| kind.severity())
}

fn has_live_borrow_at_move(
    usage: &BindingUsage,
    borrow_sites: &[KoboSpan],
    elision: Option<&CloneElisionDecision>,
) -> bool {
    if elision.is_some_and(|decision| decision.is_move()) {
        return false;
    }

    let Some(move_span) = first_move_span(usage) else {
        return false;
    };

    has_borrow_before_move(borrow_sites, move_span) && has_later_use_after_move(usage, move_span)
}

fn first_move_span(usage: &BindingUsage) -> Option<KoboSpan> {
    usage.uses.iter().find_map(|event| match event {
        UseEvent::Moved { span, .. } => Some(*span),
        _ => None,
    })
}

fn has_borrow_before_move(borrow_sites: &[KoboSpan], move_span: KoboSpan) -> bool {
    borrow_sites
        .iter()
        .any(|borrow_site| borrow_site.start <= move_span.start)
}

fn has_later_use_after_move(usage: &BindingUsage, move_span: KoboSpan) -> bool {
    usage
        .uses
        .iter()
        .filter(|event| !matches!(event, UseEvent::Moved { .. }))
        .any(|event| event.span().start > move_span.start)
}

fn is_sequential_read_only(
    read_sites: usize,
    mutable_sites: usize,
    live_borrow_at_move: bool,
    immutable_borrow_sites: &[KoboSpan],
) -> bool {
    if mutable_sites != 0 || live_borrow_at_move {
        return false;
    }

    let sequential_read_sites = read_sites >= 2 && immutable_borrow_sites.is_empty();
    let sequential_borrow_sites = immutable_borrow_sites.len() >= 2 && read_sites == 0;
    sequential_read_sites || sequential_borrow_sites
}

fn needs_sharing(
    read_sites: usize,
    borrow_count: usize,
    live_borrow_at_move: bool,
    elision: Option<&CloneElisionDecision>,
) -> bool {
    if elision.is_some_and(|decision| decision.is_move()) {
        return false;
    }

    live_borrow_at_move || borrow_count >= 2 || read_sites >= 2
}
