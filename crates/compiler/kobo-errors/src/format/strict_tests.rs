use kobo_ir::{FileId, KirNodeId, KoboSpan, StrictBoundaryFact, StrictBoundaryViolation};

use crate::codes::{KErrorCode, Severity};

use super::{render_k0041, render_k0042, render_k0043, render_k0063};

fn span(s: u32, e: u32) -> KoboSpan {
    KoboSpan::new(s, e, FileId(0))
}

fn node(id: u32) -> KirNodeId {
    KirNodeId(id)
}

// ── K0041 ──────────────────────────────────────────────────────────────

/// Test 1: render_k0041 produces K0041 with Severity::Error (Invariant C03).
#[test]
fn test_render_k0041_code_and_severity() {
    let fact = StrictBoundaryFact {
        block_span: span(10, 50),
        violation: StrictBoundaryViolation::ActiveAliases {
            binding_id: node(1),
            alias_sites: vec![span(5, 8)],
        },
    };
    let diag = render_k0041(&fact);
    assert_eq!(diag.code, KErrorCode::K0041, "K0041 code expected");
    assert_eq!(
        diag.severity,
        Severity::Error,
        "K0041 must use Severity::Error (C03)"
    );
}

/// Test 2: render_k0041 primary span is the @strict block_span.
#[test]
fn test_render_k0041_primary_span_is_block_span() {
    let block_span = span(10, 50);
    let fact = StrictBoundaryFact {
        block_span,
        violation: StrictBoundaryViolation::ActiveAliases {
            binding_id: node(2),
            alias_sites: vec![span(3, 6), span(7, 9)],
        },
    };
    let diag = render_k0041(&fact);
    assert_eq!(
        diag.primary.span, block_span,
        "K0041 primary label must span the @strict block"
    );
    // Secondary labels cover each alias site
    assert_eq!(
        diag.secondary.len(),
        2,
        "one secondary label per alias site"
    );
}

// ── K0042 ──────────────────────────────────────────────────────────────

/// Test 3: render_k0042 produces K0042 with Severity::Error (Invariant C03).
#[test]
fn test_render_k0042_code_and_severity() {
    let fact = StrictBoundaryFact {
        block_span: span(20, 80),
        violation: StrictBoundaryViolation::ClosureCapture {
            closure_span: span(30, 60),
            captured_binding_id: node(3),
            is_move_closure: false,
            captures: vec![],
        },
    };
    let diag = render_k0042(&fact);
    assert_eq!(diag.code, KErrorCode::K0042, "K0042 code expected");
    assert_eq!(
        diag.severity,
        Severity::Error,
        "K0042 must use Severity::Error (C03)"
    );
}

/// Test 4: render_k0042 primary span is the closure_span; block appears as secondary.
#[test]
fn test_render_k0042_has_secondary_block_label() {
    let block_span = span(20, 80);
    let closure_span = span(30, 60);
    let fact = StrictBoundaryFact {
        block_span,
        violation: StrictBoundaryViolation::ClosureCapture {
            closure_span,
            captured_binding_id: node(4),
            is_move_closure: false,
            captures: vec![],
        },
    };
    let diag = render_k0042(&fact);
    assert_eq!(
        diag.primary.span, closure_span,
        "K0042 primary label must span the closure"
    );
    assert!(
        diag.secondary.iter().any(|s| s.span == block_span),
        "K0042 must have a secondary label at the @strict block_span"
    );
}

// ── K0043 ──────────────────────────────────────────────────────────────

/// Test 5: render_k0043 produces K0043 with Severity::Error (Invariant C03).
#[test]
fn test_render_k0043_code_and_severity() {
    let fact = StrictBoundaryFact {
        block_span: span(10, 90),
        violation: StrictBoundaryViolation::MovedInside {
            binding_id: node(5),
            move_site: span(40, 55),
        },
    };
    let diag = render_k0043(&fact);
    assert_eq!(diag.code, KErrorCode::K0043, "K0043 code expected");
    assert_eq!(
        diag.severity,
        Severity::Error,
        "K0043 must use Severity::Error (C03)"
    );
}

/// Test 6: render_k0043 primary span is the move_site.
#[test]
fn test_render_k0043_primary_span_is_move_site() {
    let move_site = span(40, 55);
    let fact = StrictBoundaryFact {
        block_span: span(10, 90),
        violation: StrictBoundaryViolation::MovedInside {
            binding_id: node(6),
            move_site,
        },
    };
    let diag = render_k0043(&fact);
    assert_eq!(
        diag.primary.span, move_site,
        "K0043 primary label must span the move site"
    );
}

// ── K0063 ──────────────────────────────────────────────────────────────

/// Test 7: render_k0063 produces K0063 with Severity::Error (Invariant C03).
#[test]
fn test_render_k0063_code_and_severity() {
    let fact = StrictBoundaryFact {
        block_span: span(50, 100),
        violation: StrictBoundaryViolation::AsyncContext {
            async_fn_span: span(0, 120),
        },
    };
    let diag = render_k0063(&fact);
    assert_eq!(diag.code, KErrorCode::K0063, "K0063 code expected");
    assert_eq!(
        diag.severity,
        Severity::Error,
        "K0063 must use Severity::Error (C03)"
    );
}
