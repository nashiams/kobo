use kobo_ir::{FileSetBuilder, KoboSpan};

use crate::codes::{KErrorCode, Severity};
use crate::diagnostic::{DiagExplanation, DiagLabel, DiagLabelKind, KDiagnostic};

use super::format_diagnostic;

#[test]
fn formatter_renders_k0001_with_secondary_label() {
    let mut file_set_builder = FileSetBuilder::new();
    let file_id = file_set_builder.add_file(
        "src/main.kobo".into(),
        "process(config);\nlog(config);\n".to_owned(),
    );
    let file_set = file_set_builder.finish();
    let diagnostic = KDiagnostic::new(
        KErrorCode::K0001,
        Severity::Error,
        DiagLabel::primary(KoboSpan::new(21, 27, file_id), "value used here"),
        DiagExplanation("`config` was moved earlier and is no longer valid".to_owned()),
        "no automatic rewrite applied",
    )
    .with_secondary_label(DiagLabel::new(
        KoboSpan::new(8, 14, file_id),
        DiagLabelKind::Secondary,
        "value moved here",
    ))
    .with_help("clone explicitly at the call site")
    .with_run("kobo check src/main.kobo:2");

    let rendered = format_diagnostic(&file_set, &diagnostic);

    assert!(rendered.contains("error[K0001]: value used after move"));
    assert!(rendered.contains("--> src/main.kobo:2:5"));
    assert!(rendered.contains("------ value moved here"));
    assert!(rendered.contains("I found:"));
    assert!(rendered.contains("Why I care:"));
    assert!(rendered.contains("Try this:"));
    assert!(rendered.contains("Hint: clone explicitly at the call site."));
    assert!(rendered.contains("Run `kobo explain K0001`."));
    assert!(rendered.contains("Rerun with `kobo check src/main.kobo:2`."));
    assert!(!rendered.contains("Rerun:"));
}

#[test]
fn formatter_renders_k0002_and_omits_empty_help_run() {
    let mut file_set_builder = FileSetBuilder::new();
    let file_id = file_set_builder.add_file(
        "src/main.kobo".into(),
        "let r = &data;\ndata.push(1);\n".to_owned(),
    );
    let file_set = file_set_builder.finish();
    let diagnostic = KDiagnostic::new(
        KErrorCode::K0002,
        Severity::Error,
        DiagLabel::primary(KoboSpan::new(15, 19, file_id), "mutable borrow occurs here"),
        "immutable borrow is still active",
        "flagged as conflict; simultaneous borrows would panic",
    )
    .with_secondary_label(DiagLabel::new(
        KoboSpan::new(9, 13, file_id),
        DiagLabelKind::Secondary,
        "immutable borrow occurs here",
    ));

    let rendered = format_diagnostic(&file_set, &diagnostic);

    assert!(rendered.contains("error[K0002]: mutable borrow overlaps another borrow"));
    assert!(rendered.contains("---- immutable borrow occurs here"));
    assert!(!rendered.contains("help:"));
    assert!(!rendered.contains("run:"));
}

#[test]
fn formatter_falls_back_to_byte_ranges_for_invalid_spans() {
    let mut file_set_builder = FileSetBuilder::new();
    let file_id = file_set_builder.add_file("src/main.kobo".into(), "data".to_owned());
    let file_set = file_set_builder.finish();
    let diagnostic = KDiagnostic::new(
        KErrorCode::K0001,
        Severity::Error,
        DiagLabel::primary(KoboSpan::new(10, 20, file_id), "value used here"),
        "fallback expected",
        "no automatic rewrite applied",
    );

    let rendered = format_diagnostic(&file_set, &diagnostic);
    assert!(rendered.contains("[bytes 10..20]"));
    assert!(rendered.contains("src/main.kobo"));
}
