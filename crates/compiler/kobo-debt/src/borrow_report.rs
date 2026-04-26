use kobo_ir::{BindingUsage, BorrowKind, KoboSpan, SharedBindingFacts, UseEvent};
use serde::{Deserialize, Serialize};

/// How to fix a detected borrow overlap.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BorrowFixPattern {
    /// Copy the field before borrowing.
    ExtractBeforeBorrow,
    /// Access different fields to split the borrow.
    SplitBorrow,
    /// Reduce borrow scope to avoid overlap.
    ScopeNarrowing,
    /// Clone the data.
    Clone,
}

/// Classification of a borrow conflict.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BorrowConflictKind {
    /// An immutable borrow overlaps a mutable borrow.
    ReadWriteOverlap,
    /// Two or more mutable borrows alias the same binding.
    MultiMutableAlias,
    /// A borrow escapes the scope where the referent lives.
    LifetimeEscape,
}

/// A single detected overlapping borrow site.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorrowOverlap {
    pub binding_name: String,
    pub conflict_kind: BorrowConflictKind,
    pub immutable_spans: Vec<KoboSpan>,
    pub mutable_spans: Vec<KoboSpan>,
    pub fix_pattern: Option<BorrowFixPattern>,
}

/// Report of all detected borrow overlaps for one or more bindings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorrowReport {
    pub schema_version: u32,
    pub total_bindings_analyzed: usize,
    pub bindings_with_overlaps: usize,
    pub overlapping_sites: Vec<BorrowOverlap>,
}

/// Build borrow overlap report for a single binding.
pub fn build_borrow_report(
    binding_name: &str,
    usage: &BindingUsage,
    facts: &SharedBindingFacts,
) -> BorrowReport {
    let mut overlapping_sites = Vec::new();

    // Collect borrow events with their kinds and spans.
    let mut immutable_borrows: Vec<KoboSpan> = Vec::new();
    let mut mutable_borrows: Vec<KoboSpan> = Vec::new();
    let mut move_spans: Vec<KoboSpan> = Vec::new();

    for event in &usage.uses {
        match event {
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span,
            } => {
                immutable_borrows.push(*span);
            }
            UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                span,
            } => {
                mutable_borrows.push(*span);
            }
            UseEvent::Moved { span, .. } => {
                move_spans.push(*span);
            }
            _ => {}
        }
    }

    // Check immutable vs mutable overlaps (spans overlap if start < other.end && other.start < end).
    let mut has_overlap = false;
    let mut overlap_immut = Vec::new();
    let mut overlap_mut = Vec::new();

    for imm in &immutable_borrows {
        for mt in &mutable_borrows {
            if spans_overlap(*imm, *mt) {
                has_overlap = true;
                if !overlap_immut.contains(imm) {
                    overlap_immut.push(*imm);
                }
                if !overlap_mut.contains(mt) {
                    overlap_mut.push(*mt);
                }
            }
        }
    }

    if has_overlap {
        overlapping_sites.push(BorrowOverlap {
            binding_name: binding_name.to_string(),
            conflict_kind: BorrowConflictKind::ReadWriteOverlap,
            immutable_spans: overlap_immut,
            mutable_spans: overlap_mut,
            fix_pattern: Some(BorrowFixPattern::ScopeNarrowing),
        });
    }

    // Check live borrow at move.
    if facts.live_borrow_at_move {
        let all_borrows: Vec<KoboSpan> = immutable_borrows
            .iter()
            .chain(mutable_borrows.iter())
            .copied()
            .collect();
        for borrow_span in &all_borrows {
            for move_span in &move_spans {
                if spans_overlap(*borrow_span, *move_span) {
                    overlapping_sites.push(BorrowOverlap {
                        binding_name: binding_name.to_string(),
                        conflict_kind: BorrowConflictKind::LifetimeEscape,
                        immutable_spans: vec![*borrow_span],
                        mutable_spans: vec![],
                        fix_pattern: Some(BorrowFixPattern::Clone),
                    });
                }
            }
        }
    }

    BorrowReport {
        schema_version: 1,
        total_bindings_analyzed: 1,
        bindings_with_overlaps: if overlapping_sites.is_empty() { 0 } else { 1 },
        overlapping_sites,
    }
}

fn spans_overlap(a: KoboSpan, b: KoboSpan) -> bool {
    a.start < b.end && b.start < a.end
}
