use std::collections::{HashMap, HashSet};

use crate::debt::{KirStructDef, WarnEarlyFact};
use crate::node_id::{CfgBlockId, KirNodeId, KoboAstNodeId};
use crate::ownership::{OwnershipTier, TierDecision, TransformFacts};
use crate::resource::ResourceKind;
use crate::span::KoboSpan;
use crate::strict::{CaptureSet, StrictBoundaryFact, StrictFnMode};

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

/// A parse-time error or warning from a `#[kobo::relax]` attribute.
///
/// Produced by `kobo-transform`, consumed by `kobo-driver` to emit diagnostics.
/// `is_error = true` → `Severity::Error`; `false` → `Severity::Warning`.
#[derive(Debug, Clone)]
pub struct RelaxAttrError {
    pub span: KoboSpan,
    pub message: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MustCallAction {
    pub name: String,
    pub span: KoboSpan,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MustCallObligation {
    pub owner_type: String,
    pub owner_span: KoboSpan,
    pub attr_span: KoboSpan,
    pub actions: Vec<MustCallAction>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MustCallAttrError {
    pub span: KoboSpan,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldCapabilityField {
    pub name: String,
    pub mutable: bool,
    pub span: KoboSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldCapabilityView {
    pub function: String,
    pub owner: String,
    pub owner_type: Option<String>,
    pub using_span: KoboSpan,
    pub fields: Vec<FieldCapabilityField>,
}

/// The target element tagged by `#[kobo::migrate]` [G6 / R05].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MigrateTarget {
    /// `#[kobo::migrate] fn f() { ... }`
    Function,
    /// `#[kobo::migrate] let x = ...;`
    LetBinding,
    /// `#[kobo::migrate] param: T`
    Parameter,
}

/// Metadata-only site tagged for future migration [R05].
/// Populated during transform walk, consumed by `kobo debt`.
/// Has ZERO effect on codegen or runtime [Contract R05 / Trap 4].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MigrateSite {
    pub span: KoboSpan,
    pub target: MigrateTarget,
    pub reason: Option<String>,
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
    /// Struct definitions collected during the transform pass, for K0080-P detection.
    struct_defs: Vec<KirStructDef>,
    /// K0080-P structural warnings detected by the warn_early pass.
    warn_early_facts: Vec<WarnEarlyFact>,
    /// Capture sets for all @strict blocks in this file (Contract C09).
    strict_capture_sets: Vec<CaptureSet>,
    /// Boundary violation facts for all @strict blocks (Contract C09, C05).
    strict_boundary_facts: Vec<StrictBoundaryFact>,
    /// Mode for each @strict fn (Full or AsyncDeferred), keyed by fn span.
    strict_fn_modes: HashMap<KoboSpan, StrictFnMode>,
    /// Byte-offset spans of functions annotated with `#[kobo::relax]`.
    /// Used by the driver to suppress warnings from diagnostics inside these ranges [G5].
    relaxed_fn_ranges: Vec<KoboSpan>,
    /// Parse-time validation errors/warnings for `#[kobo::relax]` attributes [G5].
    relax_attr_errors: Vec<RelaxAttrError>,
    /// Sites tagged with `#[kobo::migrate]` — metadata-only, zero codegen effect [G6 / R05].
    migrate_sites: Vec<MigrateSite>,
    must_call_obligations: Vec<MustCallObligation>,
    must_call_attr_errors: Vec<MustCallAttrError>,
    field_capability_views: Vec<FieldCapabilityView>,
    /// Method name → `true` if `&mut self`, scanned from impl blocks + config.
    method_mutability: HashMap<String, bool>,
    /// S-3: KIR node IDs whose bindings belong to engine struct types.
    /// These are capped at PlainOwned by the solver/codegen.
    engine_ceiling_nodes: HashSet<KirNodeId>,
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
            struct_defs: Vec::new(),
            warn_early_facts: Vec::new(),
            strict_capture_sets: Vec::new(),
            strict_boundary_facts: Vec::new(),
            strict_fn_modes: HashMap::new(),
            relaxed_fn_ranges: Vec::new(),
            relax_attr_errors: Vec::new(),
            migrate_sites: Vec::new(),
            must_call_obligations: Vec::new(),
            must_call_attr_errors: Vec::new(),
            field_capability_views: Vec::new(),
            method_mutability: HashMap::new(),
            engine_ceiling_nodes: HashSet::new(),
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

    pub fn transform_facts(&self) -> &TransformFacts {
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

    pub fn struct_defs(&self) -> &[KirStructDef] {
        &self.struct_defs
    }

    pub fn set_struct_defs(&mut self, defs: Vec<KirStructDef>) {
        self.struct_defs = defs;
    }

    pub fn warn_early_facts(&self) -> &[WarnEarlyFact] {
        &self.warn_early_facts
    }

    pub fn set_warn_early_facts(&mut self, facts: Vec<WarnEarlyFact>) {
        self.warn_early_facts = facts;
    }

    pub fn strict_capture_sets(&self) -> &[CaptureSet] {
        &self.strict_capture_sets
    }

    pub fn set_strict_capture_sets(&mut self, sets: Vec<CaptureSet>) {
        self.strict_capture_sets = sets;
    }

    pub fn strict_boundary_facts(&self) -> &[StrictBoundaryFact] {
        &self.strict_boundary_facts
    }

    pub fn set_strict_boundary_facts(&mut self, facts: Vec<StrictBoundaryFact>) {
        self.strict_boundary_facts = facts;
    }

    pub fn strict_fn_modes(&self) -> &HashMap<KoboSpan, StrictFnMode> {
        &self.strict_fn_modes
    }

    pub fn set_strict_fn_modes(&mut self, modes: HashMap<KoboSpan, StrictFnMode>) {
        self.strict_fn_modes = modes;
    }

    pub fn relaxed_fn_ranges(&self) -> &[KoboSpan] {
        &self.relaxed_fn_ranges
    }

    pub fn set_relaxed_fn_ranges(&mut self, ranges: Vec<KoboSpan>) {
        self.relaxed_fn_ranges = ranges;
    }

    pub fn relax_attr_errors(&self) -> &[RelaxAttrError] {
        &self.relax_attr_errors
    }

    pub fn set_relax_attr_errors(&mut self, errors: Vec<RelaxAttrError>) {
        self.relax_attr_errors = errors;
    }

    pub fn migrate_sites(&self) -> &[MigrateSite] {
        &self.migrate_sites
    }

    pub fn set_migrate_sites(&mut self, sites: Vec<MigrateSite>) {
        self.migrate_sites = sites;
    }

    pub fn must_call_obligations(&self) -> &[MustCallObligation] {
        &self.must_call_obligations
    }

    pub fn set_must_call_obligations(&mut self, obligations: Vec<MustCallObligation>) {
        self.must_call_obligations = obligations;
    }

    pub fn must_call_attr_errors(&self) -> &[MustCallAttrError] {
        &self.must_call_attr_errors
    }

    pub fn set_must_call_attr_errors(&mut self, errors: Vec<MustCallAttrError>) {
        self.must_call_attr_errors = errors;
    }

    pub fn field_capability_views(&self) -> &[FieldCapabilityView] {
        &self.field_capability_views
    }

    pub fn set_field_capability_views(&mut self, views: Vec<FieldCapabilityView>) {
        self.field_capability_views = views;
    }

    pub fn method_mutability(&self) -> &HashMap<String, bool> {
        &self.method_mutability
    }

    pub fn set_method_mutability(&mut self, map: HashMap<String, bool>) {
        self.method_mutability = map;
    }

    /// S-3: Check if a node is capped to PlainOwned due to engine struct membership.
    pub fn is_engine_ceiling(&self, id: KirNodeId) -> bool {
        self.engine_ceiling_nodes.contains(&id)
    }

    /// S-3: Mark nodes as belonging to engine struct types.
    pub fn set_engine_ceiling_nodes(&mut self, nodes: HashSet<KirNodeId>) {
        self.engine_ceiling_nodes = nodes;
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
