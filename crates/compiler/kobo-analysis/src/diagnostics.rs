use kobo_errors::{resolve_severity, CliSuggestion, DiagDecision, DiagExplanation, DiagHelp, DiagLabel, KDiagnostic};
use kobo_errors::{KErrorCode, Severity};
use kobo_errors::{render_k0041, render_k0042, render_k0043, render_k0063, render_labeled_cross_boundary};
use kobo_ir::{FileSet, Kir, KoboMode, OwnershipTier, StrictBoundaryViolation, TransformFacts};

use crate::ownership_facts::{BorrowFact, BorrowKind, HintConflictFact, MoveFact};
use crate::runner::AnalysisFacts;

pub fn facts_to_diagnostics(
    facts: &AnalysisFacts,
    transform_facts: &TransformFacts,
    file_set: &FileSet,
    kir: &Kir,
    mode: KoboMode,
) -> Vec<KDiagnostic> {
    let mut diagnostics = Vec::new();

    for move_fact in &facts.moves {
        if move_fact_is_rewritten_as_plain_clone(move_fact, transform_facts) {
            continue;
        }
        // v0.6: Script mode is silent for K0001 (resolve_severity returns None) [R6-11].
        if let Some(severity) = resolve_severity(KErrorCode::K0001, mode) {
            diagnostics.push(move_fact_diagnostic(move_fact, file_set, severity));
        }
    }

    for borrow_fact in &facts.borrows {
        // v0.6: Script mode is silent for K0002 [R6-11].
        if let Some(severity) = resolve_severity(KErrorCode::K0002, mode) {
            diagnostics.push(borrow_fact_diagnostic(borrow_fact, file_set, severity));
        }
    }

    for hint_conflict in &transform_facts.hint_conflicts {
        if let Some(binding) = transform_facts.binding(hint_conflict.node) {
            // K0025 is always Error (constraint conflict, not perf advisory).
            let severity = resolve_severity(KErrorCode::K0025, mode)
                .unwrap_or(Severity::Error);
            diagnostics.push(hint_conflict_diagnostic(hint_conflict, binding, file_set, severity));
        }
    }

    // v0.5: Convert strict boundary facts → diagnostics (BUG-02 fix).
    // @strict violations are always Error — mode-independent [Contract R10].
    if !kir.strict_boundary_facts().is_empty() {
        for fact in kir.strict_boundary_facts() {
            let diag = match &fact.violation {
                StrictBoundaryViolation::ActiveAliases { .. } => render_k0041(fact),
                StrictBoundaryViolation::ClosureCapture { .. } => render_k0042(fact),
                StrictBoundaryViolation::MovedInside { .. } => render_k0043(fact),
                StrictBoundaryViolation::AsyncContext { .. } => render_k0063(fact),
                StrictBoundaryViolation::LabeledCrossBoundary { .. } => {
                    render_labeled_cross_boundary(fact)
                }
            };
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn move_fact_diagnostic(move_fact: &MoveFact, file_set: &FileSet, severity: Severity) -> KDiagnostic {
    KDiagnostic::new(
        KErrorCode::K0001,
        severity,
        DiagLabel::primary(move_fact.later_use, "value used after move"),
        move_explanation(move_fact, file_set),
        DiagDecision("flagged before lowering; no automatic rewrite applied".to_owned()),
    )
    .with_secondary_label(DiagLabel::secondary(
        move_fact.move_site,
        "value moved here",
    ))
    .with_help(DiagHelp(
        "if you want both calls to share the same value, clone explicitly at the move site"
            .to_owned(),
    ))
    .with_run(CliSuggestion(run_target(file_set, move_fact.move_site)))
}

fn move_fact_is_rewritten_as_plain_clone(
    move_fact: &MoveFact,
    transform_facts: &TransformFacts,
) -> bool {
    transform_facts.iter_bindings().any(|binding| {
        binding.plain_clone_alias
            && binding.plain_clone_source == Some(move_fact.binding)
            && binding.plain_clone_move_span == Some(move_fact.move_site)
    })
}

fn borrow_fact_diagnostic(borrow_fact: &BorrowFact, file_set: &FileSet, severity: Severity) -> KDiagnostic {
    KDiagnostic::new(
        KErrorCode::K0002,
        severity,
        DiagLabel::primary(borrow_fact.conflict_site, "mutable borrow occurs here"),
        borrow_explanation(borrow_fact, file_set),
        DiagDecision("flagged as conflict; simultaneous borrows would panic at runtime".to_owned()),
    )
    .with_secondary_label(DiagLabel::secondary(
        borrow_fact.borrow_site,
        earlier_borrow_label(borrow_fact.borrow_kind),
    ))
    .with_help(DiagHelp(
        "restructure the code so the earlier borrow ends before mutation".to_owned(),
    ))
    .with_run(CliSuggestion(run_target(file_set, borrow_fact.borrow_site)))
}

fn hint_conflict_diagnostic(
    hint_conflict: &HintConflictFact,
    binding: &kobo_ir::TransformBindingFacts,
    file_set: &FileSet,
    severity: Severity,
) -> KDiagnostic {
    KDiagnostic::new(
        KErrorCode::K0025,
        severity,
        DiagLabel::primary(
            hint_conflict.hint_span,
            format!(
                "hint requests {} ownership here",
                hint_conflict.hint.as_str()
            ),
        ),
        hint_conflict_explanation(hint_conflict, binding),
        DiagDecision(format!(
            "assigned {} (greedy priority {}) after the hinted candidate failed",
            hint_conflict.chosen_tier.label(),
            tier_priority(hint_conflict.chosen_tier)
        )),
    )
    .with_secondary_label(DiagLabel::secondary(
        binding.span,
        "hint applies to this binding",
    ))
    .with_help(DiagHelp(
        "drop the later alias, or change the hint to match the actual usage pattern".to_owned(),
    ))
    .with_run(CliSuggestion(run_target(file_set, hint_conflict.hint_span)))
}

fn move_explanation(move_fact: &MoveFact, file_set: &FileSet) -> DiagExplanation {
    if move_fact.move_site == move_fact.later_use {
        return DiagExplanation(
            "value was moved while another ownership obligation was still active".to_owned(),
        );
    }

    let binding_name = binding_name_at(file_set, move_fact.later_use);
    let move_line = source_line(file_set, move_fact.move_site);
    DiagExplanation(format!(
        "`{binding_name}` was moved on line {move_line}; the original binding is no longer valid at this later use"
    ))
}

fn borrow_explanation(borrow_fact: &BorrowFact, file_set: &FileSet) -> DiagExplanation {
    let binding_name = binding_name_at(file_set, borrow_fact.borrow_site);
    let borrow_line = source_line(file_set, borrow_fact.borrow_site);
    let borrow_mode = borrow_mode_text(borrow_fact.borrow_kind);
    DiagExplanation(format!(
        "`{binding_name}` is already borrowed {borrow_mode} on line {borrow_line}; simultaneous mutable borrow would panic at runtime"
    ))
}

fn hint_conflict_explanation(
    hint_conflict: &kobo_ir::HintConflictFact,
    binding: &kobo_ir::TransformBindingFacts,
) -> DiagExplanation {
    let conflict = match hint_conflict.reason {
        kobo_ir::HintConflictReason::Aliasing => {
            "an active borrow is still live at the later move site"
        }
        kobo_ir::HintConflictReason::MutableUse => {
            "mutable borrowing requires the shared-mutable floor"
        }
        kobo_ir::HintConflictReason::SendRequired => {
            "thread-crossing usage requires a Send-safe shared tier"
        }
        kobo_ir::HintConflictReason::AsyncDeferred => "Box<T> is deferred inside async fn in v0.3",
        kobo_ir::HintConflictReason::SharedUsage => {
            "multiple read-only borrow sites still require sharing in script mode"
        }
        kobo_ir::HintConflictReason::Unknown => "later usage invalidated the hinted candidate",
    };

    DiagExplanation(format!(
        "hint `ownership = \"{}\"` conflicts with {} for `{}`",
        hint_conflict.hint.as_str(),
        conflict,
        binding.binding_name,
    ))
}

fn binding_name_at(file_set: &FileSet, span: kobo_ir::KoboSpan) -> String {
    file_set
        .get(span.file_id)
        .and_then(|file| file.snippet(span))
        .unwrap_or("value")
        .to_owned()
}

fn source_line(file_set: &FileSet, span: kobo_ir::KoboSpan) -> usize {
    file_set
        .get(span.file_id)
        .map(|file| file.line_col(span.start).0)
        .unwrap_or(1)
}

fn earlier_borrow_label(borrow_kind: BorrowKind) -> &'static str {
    match borrow_kind {
        BorrowKind::Immutable => "immutable borrow occurs here",
        BorrowKind::Mutable => "mutable borrow occurs here",
    }
}

fn borrow_mode_text(borrow_kind: BorrowKind) -> &'static str {
    match borrow_kind {
        BorrowKind::Immutable => "immutably",
        BorrowKind::Mutable => "mutably",
    }
}

fn run_target(file_set: &FileSet, span: kobo_ir::KoboSpan) -> String {
    let Some(file) = file_set.get(span.file_id) else {
        return "kobo check".to_owned();
    };
    let (line, _) = file.line_col(span.start);
    format!("kobo check {}:{line}", file.path.display())
}

fn tier_priority(tier: OwnershipTier) -> usize {
    tier.greedy_priority()
}
