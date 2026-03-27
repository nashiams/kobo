use kobo_errors::{CliSuggestion, DiagDecision, DiagExplanation, DiagHelp, DiagLabel, KDiagnostic};
use kobo_errors::{KErrorCode, Severity};
use kobo_ir::FileSet;

use crate::ownership_facts::{BorrowFact, BorrowKind, MoveFact};
use crate::runner::AnalysisFacts;

pub fn facts_to_diagnostics(facts: &AnalysisFacts, file_set: &FileSet) -> Vec<KDiagnostic> {
    let mut diagnostics = Vec::new();

    for move_fact in &facts.moves {
        diagnostics.push(move_fact_diagnostic(move_fact, file_set));
    }

    for borrow_fact in &facts.borrows {
        diagnostics.push(borrow_fact_diagnostic(borrow_fact, file_set));
    }

    diagnostics
}

fn move_fact_diagnostic(move_fact: &MoveFact, file_set: &FileSet) -> KDiagnostic {
    KDiagnostic::new(
        KErrorCode::K0001,
        Severity::Error,
        DiagLabel::primary(move_fact.later_use, "value used after move"),
        move_explanation(move_fact, file_set),
        DiagDecision("flagged before lowering; no automatic rewrite applied".to_owned()),
    )
    .with_secondary_label(DiagLabel::secondary(move_fact.move_site, "value moved here"))
    .with_help(DiagHelp(
        "if you want both calls to share the same value, clone explicitly at the move site"
            .to_owned(),
    ))
    .with_run(CliSuggestion(run_target(file_set, move_fact.move_site)))
}

fn borrow_fact_diagnostic(borrow_fact: &BorrowFact, file_set: &FileSet) -> KDiagnostic {
    KDiagnostic::new(
        KErrorCode::K0002,
        Severity::Error,
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
