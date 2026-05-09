use kobo_errors::KErrorCode;
use kobo_ir::{FileId, NodeIdGen};
use kobo_parser::{parse_file_recovering, RecoveryMode};

#[test]
fn recovering_parse_reports_two_independent_syntax_errors() {
    let source = r#"
fn first() {
    let x =
}

fn second() {
    let y =
}
"#;

    let mut ids = NodeIdGen::new();
    let outcome = parse_file_recovering(source, FileId(0), &mut ids, RecoveryMode::Recover);

    assert!(
        outcome.diagnostics.len() >= 2,
        "expected independent syntax errors to produce multiple diagnostics, got {:?}",
        outcome.diagnostics
    );
    assert!(
        outcome.poisoned_spans.len() >= 2,
        "syntax errors should produce poisoned spans"
    );
    assert!(
        outcome.diagnostics.iter().all(|diag| matches!(
            diag.code,
            KErrorCode::K0100 | KErrorCode::K0101 | KErrorCode::K0102 | KErrorCode::K0103
        )),
        "recovery diagnostics must use parser K010x codes: {:?}",
        outcome.diagnostics
    );
    assert!(
        outcome.poisoned_spans.iter().all(|span| {
            span.file_id == FileId(0)
                && span.start < span.end
                && (span.end as usize) <= source.len()
        }),
        "poisoned spans must stay in parse-input coordinates"
    );
}

#[test]
fn strict_parse_mode_preserves_old_fail_fast_behavior() {
    let source = "fn main( {";
    let mut ids = NodeIdGen::new();
    let outcome = parse_file_recovering(source, FileId(0), &mut ids, RecoveryMode::FailFast);

    assert!(outcome.file.is_none());
    assert_eq!(outcome.diagnostics.len(), 1);
}
