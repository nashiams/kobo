use kobo_ir::{StrictBoundaryFact, StrictBoundaryViolation};

use crate::diagnostic::{DiagLabel, KDiagnostic};
// ---------------------------------------------------------------------------
// @strict boundary diagnostics (K0041 / K0042 / K0043 / K0063)
// ---------------------------------------------------------------------------

/// Render a K0041 diagnostic: active Rc aliases at @strict block entry.
///
/// Invariant C03: Severity::Error.
pub fn render_k0041(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let alias_sites = match &fact.violation {
        StrictBoundaryViolation::ActiveAliases { alias_sites, .. } => alias_sites.as_slice(),
        _ => return render_wrong_strict_violation(KErrorCode::K0041, fact, "active aliases"),
    };

    let primary = DiagLabel::primary(fact.block_span, "active aliases at @strict block entry");

    let alias_count = alias_sites.len();
    let explanation = format!(
        "value has {alias_count} active alias{} when the @strict block starts; \
         all aliases must end before the boundary is entered",
        if alias_count == 1 { "" } else { "es" }
    );

    let mut diag = KDiagnostic::new(
        KErrorCode::K0041,
        Severity::Error,
        primary,
        explanation,
        "drop or explicitly clone-then-drop aliases before the @strict { } boundary",
    );

    for &alias_span in alias_sites {
        diag.secondary
            .push(DiagLabel::secondary(alias_span, "alias created here"));
    }

    diag
}

/// Render a K0042 diagnostic: closure captures Rc<RefCell<T>> across @strict boundary.
///
/// Invariant C03: Severity::Error.
pub fn render_k0042(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let closure_span = match &fact.violation {
        StrictBoundaryViolation::ClosureCapture { closure_span, .. } => *closure_span,
        _ => return render_wrong_strict_violation(KErrorCode::K0042, fact, "closure capture"),
    };

    let primary = DiagLabel::primary(
        closure_span,
        "closure captures a shared mutable binding across @strict boundary",
    );

    let explanation =
        "a closure defined inside an @strict block captures a shared mutable binding; \
         calling the closure later could keep the strict borrow alive after the block ends";

    let mut diag = KDiagnostic::new(
        KErrorCode::K0042,
        Severity::Error,
        primary,
        explanation,
        "extract the closure outside the @strict block, or refactor to avoid capturing \
         the guarded value",
    );

    diag.secondary
        .push(DiagLabel::secondary(fact.block_span, "@strict block here"));
    diag
}

/// Render a K0043 diagnostic: value moved inside @strict block.
///
/// Invariant C03: Severity::Error.
pub fn render_k0043(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let move_site = match &fact.violation {
        StrictBoundaryViolation::MovedInside { move_site, .. } => *move_site,
        _ => return render_wrong_strict_violation(KErrorCode::K0043, fact, "moved value"),
    };

    let primary = DiagLabel::primary(move_site, "value moved here");

    let explanation = "a binding is moved inside an @strict block; the boundary guard is dropped \
         on exit from the block and the moved value cannot be restored to its outer ownership shape";

    let mut diag = KDiagnostic::new(
        KErrorCode::K0043,
        Severity::Error,
        primary,
        explanation,
        "clone the value before moving, or drop the guard before the move statement",
    );

    diag.secondary
        .push(DiagLabel::secondary(fact.block_span, "@strict block"));
    diag
}

/// Render a K0063 diagnostic: @strict block inside async fn.
///
/// Invariant C03: Severity::Error.
pub fn render_k0063(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let async_fn_span = match &fact.violation {
        StrictBoundaryViolation::AsyncContext { async_fn_span } => *async_fn_span,
        _ => return render_wrong_strict_violation(KErrorCode::K0063, fact, "async context"),
    };

    let primary = DiagLabel::primary(fact.block_span, "this strict borrow starts here");

    let explanation =
        "Kobo needs strict borrows to end before the function can pause or be cancelled.";

    let mut diag = KDiagnostic::new(
        KErrorCode::K0063,
        Severity::Error,
        primary,
        explanation,
        "1. Use `@strict async fn` if the whole function should follow Kobo's async strict rules.\n\
         2. Or move this strict work into a small non-async helper.",
    )
    .with_finding("This strict block runs inside an async function.");

    diag.secondary.push(DiagLabel::secondary(
        async_fn_span,
        "async functions can pause at `.await`",
    ));
    diag
}

/// Render a labeled break/continue crossing @strict boundary diagnostic.
///
/// Render K0044 for labeled break/continue across an @strict boundary.
/// Invariant C03: Severity::Error.
pub fn render_labeled_cross_boundary(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let (label, break_span) = match &fact.violation {
        StrictBoundaryViolation::LabeledCrossBoundary {
            label,
            break_or_continue_span,
            ..
        } => (label.as_str(), *break_or_continue_span),
        _ => {
            return render_wrong_strict_violation(
                KErrorCode::K0044,
                fact,
                "labeled break or continue",
            )
        }
    };

    let primary = DiagLabel::primary(
        break_span,
        format!("labeled `{label}` crosses @strict boundary"),
    );

    let explanation = format!(
        "break or continue to label `{label}` would exit the @strict block without \
         dropping borrow guards in the correct order; this is not allowed in strict code"
    );

    let mut diag = KDiagnostic::new(
        KErrorCode::K0044,
        Severity::Error,
        primary,
        explanation,
        "restructure the code to avoid labeled break/continue across @strict boundaries",
    );

    diag.secondary
        .push(DiagLabel::secondary(fact.block_span, "@strict block here"));
    diag
}

fn render_wrong_strict_violation(
    code: crate::codes::KErrorCode,
    fact: &StrictBoundaryFact,
    expected: &str,
) -> KDiagnostic {
    use crate::codes::Severity;

    KDiagnostic::new(
        code,
        Severity::Error,
        DiagLabel::primary(fact.block_span, "strict diagnostic route mismatch"),
        format!(
            "internal Kobo routing expected {expected}, but received a different strict boundary fact"
        ),
        "report this as a Kobo compiler bug; the source program should not make the renderer panic",
    )
    .with_finding("Kobo reached a diagnostic renderer with the wrong strict fact type.")
}
