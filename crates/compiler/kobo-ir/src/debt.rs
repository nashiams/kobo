use crate::kir::MigrateSite;
use crate::node_id::KirNodeId;
use crate::ownership::OwnershipTier;
use crate::span::KoboSpan;

// --- Warning-early pattern types ---

/// A detected structural ownership pattern that will require an architectural
/// decision at migration time. Produced by `kobo-transform`'s warn_early pass
/// and consumed by the driver (which builds K0080-P diagnostics) and
/// `kobo-debt` (which formats the Structural warnings section).
///
/// The type lives in `kobo-ir` so it can be read by both the driver and
/// `kobo-debt` without introducing a dependency between those crates.
/// This must remain in `kobo-ir` only.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WarnEarlyFact {
    pub node_id: KirNodeId,
    pub span: KoboSpan,
    pub pattern: WarnEarlyPattern,
    /// `true` when `#[kobo::known_debt = "..."]` suppresses this advisory.
    pub suppressed: bool,
    /// The reason string from the attribute. `None` when not suppressed.
    pub known_debt_reason: Option<String>,
    /// Struct name associated with the pattern (for display).
    pub struct_name: String,
}

/// The four structural ownership patterns Kobo detects for debt reporting.
///
/// K0080-P codes are always `note` severity — never `warning` or `error`.
/// They are advisory only.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WarnEarlyPattern {
    /// K0080-P1: bidirectional `Rc<RefCell<T>>` links — Rc cycle will leak.
    ///
    /// `field_pairs: Vec<(String, String)>` — each tuple is (field_a, field_b),
    /// one per bidirectional link pair. A struct with 3 back-pointer fields has
    /// 3 pairs, not 1. Never use a fixed-size array here (Rebuttal 7).
    BidirectionalRcLinks {
        struct_name: String,
        field_pairs: Vec<(String, String)>,
    },
    /// K0080-P2: parent↔child back-pointer tree — same-type Vec + Option fields.
    ///
    /// Detected structurally (same inner type on both fields), not nominally.
    /// Supersedes P1 when both patterns match the same struct.
    ///
    /// Spec originally defined a single `back_field`. Implementation uses
    /// separate `children_field` + `parent_field` for richer diagnostic
    /// messages. This is an intentional enhancement over the spec.
    ParentChildBackPointer {
        struct_name: String,
        /// Field holding `Vec<Rc<RefCell<Self>>>` (children direction).
        children_field: String,
        /// Field holding `Option<Rc<RefCell<Self>>>` (parent direction).
        parent_field: String,
    },
    /// K0080-P3: shared mutable state mutated from 3+ distinct call sites.
    ///
    /// Count is distinct caller _function_ boundaries, not raw mutation events.
    /// A loop in one function that mutates 100 times counts as 1 site.
    SharedMutableAt3PlusSites {
        binding_id: KirNodeId,
        /// Number of distinct caller function identities.
        site_count: usize,
    },
    /// K0080-P4: self-referential struct without Rc/Box indirection.
    ///
    /// `struct Chain { next: Chain }` is infinite size without Box/Rc wrapping.
    SelfReferentialStruct { struct_name: String },
}

// --- Debt complexity tiers ---

/// Migration complexity estimate for a single `RcMutShared` site.
///
/// These are estimates based on structural heuristics. Solver-backed decisions
/// produce exact classifications. The "estimate" suffix marks non-finality.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DebtComplexityTier {
    /// Tier 1: mechanical fix, expected to be auto-fixable by `kobo migrate --apply`.
    Tier1,
    /// Tier 2: design decision required (K0080-P1/P2/P3 present, or 2+ mutation sites).
    Tier2,
    /// Tier 3: unknown pattern; manual review needed (K0080-P4 or unsolvable escape).
    Tier3,
}

/// Per-site migration record for a single `RcMutShared` KIR node.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DebtSiteRecord {
    pub node_id: KirNodeId,
    #[serde(rename = "source_span")]
    pub span: KoboSpan,
    pub tier: OwnershipTier,
    /// Structural complexity estimate. Not a solver result.
    pub complexity_estimate: DebtComplexityTier,
    /// All K0080-P pattern variants associated with this site.
    #[serde(rename = "patterns")]
    pub warn_early: Vec<WarnEarlyPattern>,
    /// `true` when `#[kobo::known_debt]` suppresses all K0080-P notes for this site.
    pub suppressed: bool,
    pub binding_name: String,
    /// Annotation when binding is covered by an @strict block.
    pub strict_annotation: Option<String>,
}

// --- Wrapper inventory types ---

/// Counts of KIR nodes per ownership tier in a compiled source.
///
/// Populated by walking `OwnershipTier` nodes in the frozen KIR.
/// Never derived from text search over generated `.rs` files.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct WrapperInventory {
    pub plain_owned: usize,
    pub box_owned: usize,
    pub rc_shared: usize,
    pub arc_shared: usize,
    pub rc_mut_shared: usize,
}

/// Counts of `RcMutShared` sites per complexity tier.
///
/// Invariant: `tier1 + tier2 + tier3 == inventory.rc_mut_shared`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ComplexityBreakdown {
    pub tier1: usize,
    pub tier2: usize,
    pub tier3: usize,
}

/// A K0080-P site that was suppressed by `#[kobo::known_debt = "reason"]`.
///
/// Listed under "Acknowledged debt:" in `kobo debt` output, separate from
/// active structural warnings.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AcknowledgedDebtRecord {
    pub node_id: KirNodeId,
    pub span: KoboSpan,
    pub reason: String,
    pub pattern: WarnEarlyPattern,
    pub struct_name: String,
}

// --- Top-level debt report ---

/// Complete ownership cost report for a compiled source tree.
///
/// Built from frozen KIR by `kobo-debt::build_debt_report()`.
/// The ownership debt JSON schema uses `schema_version = 1`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DebtReport {
    /// Always `1`. Bump on breaking JSON schema changes.
    pub schema_version: u32,
    pub file_count: usize,
    pub line_count: usize,
    #[serde(rename = "wrapper_inventory")]
    pub inventory: WrapperInventory,
    #[serde(rename = "complexity_breakdown")]
    pub complexity: ComplexityBreakdown,
    /// Active (non-suppressed) structural warnings.
    pub warn_early: Vec<WarnEarlyFact>,
    /// Suppressed sites with their user-provided reason strings.
    pub acknowledged: Vec<AcknowledgedDebtRecord>,
    /// Per-site records for each `RcMutShared` node.
    pub sites: Vec<DebtSiteRecord>,
    /// Sites tagged with `#[kobo::migrate]`; metadata-only.
    pub migrate_tagged: Vec<MigrateSite>,
}

impl DebtReport {
    pub fn new() -> Self {
        Self {
            schema_version: 1,
            file_count: 0,
            line_count: 0,
            inventory: WrapperInventory::default(),
            complexity: ComplexityBreakdown::default(),
            warn_early: Vec::new(),
            acknowledged: Vec::new(),
            sites: Vec::new(),
            migrate_tagged: Vec::new(),
        }
    }
}

impl Default for DebtReport {
    fn default() -> Self {
        Self::new()
    }
}

// --- Struct type info for K0080-P detection ---

/// Simplified field type shape for K0080-P structural pattern detection.
///
/// Only the shapes relevant to K0080-P1 through P4 are represented.
/// All other types collapse to `Other`.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldTypeShape {
    /// `Rc<RefCell<TypeName>>`
    RcRefCellOf(String),
    /// `Option<Rc<RefCell<TypeName>>>`
    OptionRcRefCellOf(String),
    /// `Vec<Rc<RefCell<TypeName>>>`
    VecRcRefCellOf(String),
    /// A direct (non-indirected) named type reference, e.g. `Node` in `next: Node`.
    DirectNamed(String),
    /// `Vec<TypeName>` (direct, not through Rc/Box).
    VecDirectNamed(String),
    /// Any other type (pointers, primitives, Box, Arc, etc.).
    Other,
}

/// Metadata for a single struct field, simplified for pattern detection.
#[derive(Debug, Clone, PartialEq)]
pub struct KirStructFieldDef {
    pub name: String,
    pub shape: FieldTypeShape,
}

/// Metadata for a struct definition, collected during the transform pass.
///
/// Stored in `Kir.struct_defs` for use by `detect_warn_early` without
/// needing to re-parse the AST.
#[derive(Debug, Clone, PartialEq)]
pub struct KirStructDef {
    pub name: String,
    pub span: KoboSpan,
    pub fields: Vec<KirStructFieldDef>,
    /// Reason string from `#[kobo::known_debt = "..."]` if present.
    pub known_debt_reason: Option<String>,
    /// Span of the `#[kobo::known_debt]` attribute, for error reporting.
    pub known_debt_span: Option<KoboSpan>,
    /// Parse error from `#[kobo::known_debt]` if malformed (missing reason or
    /// empty reason string). The driver converts this to a `KDiagnostic`.
    pub known_debt_parse_error: Option<String>,
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_id::{FileId, KirNodeId};
    use crate::span::KoboSpan;

    fn dummy_span() -> KoboSpan {
        KoboSpan::new(0, 1, FileId(0))
    }

    #[test]
    fn complexity_breakdown_field_pairs_vec_not_array() {
        // Verify Vec<(String, String)> is used, not a fixed-size array.
        // This catches regression of Rebuttal 7 (P1 multi-pair truncation).
        let pattern = WarnEarlyPattern::BidirectionalRcLinks {
            struct_name: "GraphNode".to_owned(),
            field_pairs: vec![
                ("parent".to_owned(), "left".to_owned()),
                ("parent".to_owned(), "right".to_owned()),
                ("left".to_owned(), "right".to_owned()),
            ],
        };
        if let WarnEarlyPattern::BidirectionalRcLinks { field_pairs, .. } = &pattern {
            assert_eq!(field_pairs.len(), 3, "all three pairs must be preserved");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn debt_report_schema_version_is_one() {
        let report = DebtReport::new();
        assert_eq!(report.schema_version, 1, "JSON schema version must be 1");
    }

    #[test]
    fn complexity_breakdown_default_is_zero() {
        let breakdown = ComplexityBreakdown::default();
        assert_eq!(breakdown.tier1 + breakdown.tier2 + breakdown.tier3, 0);
    }

    #[test]
    fn warn_early_fact_suppressed_carries_reason() {
        let fact = WarnEarlyFact {
            node_id: KirNodeId(0),
            span: dummy_span(),
            pattern: WarnEarlyPattern::SelfReferentialStruct {
                struct_name: "Chain".to_owned(),
            },
            suppressed: true,
            known_debt_reason: Some("will use arena".to_owned()),
            struct_name: "Chain".to_owned(),
        };
        assert!(fact.suppressed);
        assert_eq!(fact.known_debt_reason.as_deref(), Some("will use arena"));
    }
}
