/// @strict IR types.
///
/// All types related to @strict blocks live here (Contract C09).
/// No @strict types are defined in kobo-transform or kobo-codegen.
use crate::node_id::KirNodeId;
use crate::span::KoboSpan;

/// How a binding is accessed inside the @strict block.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive] // F-06: v0.6 may add ReadMut, Conditional
pub enum CaptureAccessKind {
    Read,  // .borrow() guard
    Write, // .borrow_mut() guard
}

/// A single binding captured by an @strict block.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedBinding {
    pub binding_id: KirNodeId,
    pub name: String,
    pub access_kind: CaptureAccessKind,
    pub access_count: usize,         // F-06: for v0.6 suggestion quality scoring
    pub access_spans: Vec<KoboSpan>, // F-06: for v0.6 access-site clustering
}

/// Nested @strict block info, recorded before flattening (F-07).
#[derive(Debug, Clone, PartialEq)]
pub struct NestedStrictBlock {
    pub span: KoboSpan,
    pub original_captures: Vec<KirNodeId>,
}

/// The complete capture set for one @strict block.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSet {
    pub block_span: KoboSpan,
    pub bindings: Vec<CapturedBinding>,

    // AST analysis flags (produced by transform, consumed by codegen)
    pub has_question_mark: bool,
    pub has_break: bool,
    pub has_continue: bool,
    pub is_inside_loop: bool,

    // Nested flattening info (F-07: preserved for v0.6 per-block granularity)
    pub nested_blocks: Vec<NestedStrictBlock>,
}

/// How a closure captures a binding (F-01: full capture metadata for v0.6).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive] // F-01
pub enum ClosureCaptureMode {
    ByRef,
    ByRefMut,
    ByValue,
}

/// Detail about one captured binding inside a closure (F-01).
#[derive(Debug, Clone, PartialEq)]
pub struct ClosureCaptureDetail {
    pub binding_id: KirNodeId,
    pub binding_name: String,
    pub capture_mode: ClosureCaptureMode,
    pub is_rc_mut_shared: bool,
    pub usage_inside_closure: Vec<KoboSpan>,
}

/// What went wrong at the @strict boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum StrictBoundaryViolation {
    /// K0041: active aliases (Rc clones) exist at @strict entry.
    ActiveAliases {
        binding_id: KirNodeId,
        alias_sites: Vec<KoboSpan>,
    },

    /// K0042: closure captures a wrapped binding across @strict boundary.
    ClosureCapture {
        closure_span: KoboSpan,
        captured_binding_id: KirNodeId,
        is_move_closure: bool,                // F-01
        captures: Vec<ClosureCaptureDetail>, // F-01
    },

    /// K0043: value moved inside @strict block.
    MovedInside {
        binding_id: KirNodeId,
        move_site: KoboSpan,
    },

    /// K0063: @strict block inside async context without @strict async fn.
    AsyncContext { async_fn_span: KoboSpan },

    /// Labeled break/continue crosses @strict boundary (R-13, Trap 22).
    LabeledCrossBoundary {
        label: String,
        break_or_continue_span: KoboSpan,
        target_label_span: KoboSpan,
    },
}

/// A boundary fact emitted by the transform for the driver to convert into
/// a KDiagnostic. Contract C05: transform emits facts, not diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct StrictBoundaryFact {
    pub block_span: KoboSpan,
    pub violation: StrictBoundaryViolation,
}

/// Mode for @strict function lowering (F-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrictFnMode {
    /// Full @strict: parameters are not wrapped, body is an implicit @strict block.
    Full,
    /// Async function: @strict marker stripped, lowering deferred to v0.7.
    AsyncDeferred,
}
