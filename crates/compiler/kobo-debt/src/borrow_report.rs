use kobo_ir::{BindingUsage, BorrowKind, KoboSpan, SharedBindingFacts, UseEvent};

/// How to fix a detected borrow overlap.
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// A single detected overlapping borrow site.
#[derive(Clone, Debug)]
pub struct BorrowOverlap {
    pub binding_name: String,
    pub immutable_spans: Vec<KoboSpan>,
    pub mutable_spans: Vec<KoboSpan>,
    pub fix_pattern: Option<BorrowFixPattern>,
}

/// Report of all detected borrow overlaps for one binding.
#[derive(Clone, Debug)]
pub struct BorrowReport {
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
            UseEvent::Borrowed { kind: BorrowKind::Immutable, span } => {
                immutable_borrows.push(*span);
            }
            UseEvent::Borrowed { kind: BorrowKind::Mutable, span } => {
                mutable_borrows.push(*span);
            }
            UseEvent::Moved { span } => {
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
            immutable_spans: overlap_immut,
            mutable_spans: overlap_mut,
            fix_pattern: Some(BorrowFixPattern::ScopeNarrowing),
        });
    }

    // Check live borrow at move.
    if facts.live_borrow_at_move {
        let all_borrows: Vec<KoboSpan> = immutable_borrows.iter()
            .chain(mutable_borrows.iter())
            .copied()
            .collect();
        for borrow_span in &all_borrows {
            for move_span in &move_spans {
                if spans_overlap(*borrow_span, *move_span) {
                    overlapping_sites.push(BorrowOverlap {
                        binding_name: binding_name.to_string(),
                        immutable_spans: vec![*borrow_span],
                        mutable_spans: vec![],
                        fix_pattern: Some(BorrowFixPattern::Clone),
                    });
                }
            }
        }
    }

    BorrowReport { overlapping_sites }
}

fn spans_overlap(a: KoboSpan, b: KoboSpan) -> bool {
    a.start < b.end && b.start < a.end
}
