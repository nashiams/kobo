use kobo_codegen::{KoboSourceMap, RsSpan, SourceMapEntry};
use kobo_ir::{FileId, KoboSpan};

use super::remap_rustc_output;

fn sample_map() -> KoboSourceMap {
    KoboSourceMap {
        version: 3,
        file: "src/main.rs".to_owned(),
        sources: vec!["src/main.kobo".to_owned()],
        x_kobo_mappings: vec![
            SourceMapEntry {
                id: "map-1".to_owned(),
                core_event_id: None,
                binding_name: "left".to_owned(),
                rs_span: RsSpan {
                    line: 2,
                    column_start: 0,
                    column_end: 20,
                },
                kobo_span: KoboSpan::new(10, 16, FileId(0)),
                ownership_tier: "rc_refcell".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            },
            SourceMapEntry {
                id: "map-2".to_owned(),
                core_event_id: None,
                binding_name: "right".to_owned(),
                rs_span: RsSpan {
                    line: 3,
                    column_start: 0,
                    column_end: 20,
                },
                kobo_span: KoboSpan::new(20, 26, FileId(0)),
                ownership_tier: "rc_refcell".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            },
        ],
        runtime_evidence: None,
        solver_evidence: None,
        lowering_trace: Vec::new(),
    }
}

#[test]
fn remapper_handles_multi_span_errors() {
    let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":false,"label":"immutable borrow occurs here"},{"file_name":"src/main.rs","line_start":3,"column_start":1,"line_end":3,"column_end":5,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].secondary.len(), 1);
    assert_eq!(
        diagnostics[0].run.as_ref().map(|run| run.0.as_str()),
        Some("kobo inspect src/main.kobo")
    );
}

#[test]
fn remapper_handles_single_span_errors() {
    let raw = r#"{"message":"type mismatch","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].primary.text, "expected type");
    assert!(diagnostics[0].secondary.is_empty());
}

#[test]
fn remapper_falls_back_when_no_spans_map() {
    let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/main.rs","line_start":40,"column_start":1,"line_end":40,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert!(diagnostics[0]
        .explanation
        .0
        .starts_with("[remapping unavailable]"));
    assert_eq!(diagnostics[0].primary.span, KoboSpan::new(0, 0, FileId(0)));
    assert!(
        diagnostics[0]
            .primary
            .text
            .contains("compiler output could not be remapped"),
        "fallback label should mention remapping failure"
    );
    assert!(
        diagnostics[0].primary.text.contains("line 40"),
        "fallback label should include generated line info"
    );
}

#[test]
fn remapper_preserves_help_children() {
    let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[{"message":"consider borrowing","code":null,"level":"help","spans":[],"children":[]}]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert_eq!(
        diagnostics[0].help.as_ref().map(|help| help.0.as_str()),
        Some("consider borrowing")
    );
}

#[test]
fn remapper_classifies_all_rust_ownership_errors_as_ownership_debt() {
    for rustc_code in ["E0382", "E0499", "E0502", "E0505", "E0507", "E0515"] {
        let raw = format!(
            r#"{{"message":"ownership failure","code":{{"code":"{rustc_code}"}},"level":"error","spans":[{{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"ownership failure here"}}],"children":[]}}"#
        );
        let diagnostics = remap_rustc_output(&raw, &sample_map(), FileId(0));
        let diagnostic = &diagnostics[0];

        assert_eq!(diagnostic.code, kobo_errors::KErrorCode::K0099);
        assert!(
            diagnostic
                .decision
                .0
                .contains("ownership error escaped Kobo analysis"),
            "{rustc_code} should be ownership debt: {:?}",
            diagnostic.decision
        );
        assert!(
            diagnostic
                .hint
                .as_ref()
                .is_some_and(|hint| hint.0.contains("ownership debt")),
            "{rustc_code} should carry an ownership debt hint"
        );
    }
}

#[test]
fn remapper_does_not_classify_unrelated_rust_errors_as_ownership_debt() {
    let raw = r#"{"message":"type mismatch","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert!(
        diagnostics[0].hint.is_none(),
        "non-ownership Rust errors should not become ownership debt"
    );
    assert!(
        !diagnostics[0].decision.0.contains("ownership debt"),
        "non-ownership Rust errors should keep generic remap wording"
    );
}

#[test]
fn remapper_preserves_note_child_spans() {
    let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":3,"column_start":1,"line_end":3,"column_end":5,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[{"message":"immutable borrow later used here","code":null,"level":"note","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"immutable borrow occurs here"}],"children":[]}]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert_eq!(diagnostics[0].secondary.len(), 1);
    assert_eq!(
        diagnostics[0].secondary[0].text,
        "immutable borrow occurs here"
    );
}

#[test]
fn remapper_handles_partial_remapping() {
    let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":false,"label":"immutable borrow occurs here"},{"file_name":"src/main.rs","line_start":40,"column_start":1,"line_end":40,"column_end":5,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0]
        .explanation
        .0
        .starts_with("[remapping unavailable]"));
    assert_eq!(diagnostics[0].secondary.len(), 1);
}

#[test]
fn remapper_does_not_leak_rs_paths_in_fallback_labels() {
    let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/generated.rs","line_start":40,"column_start":1,"line_end":40,"column_end":5,"is_primary":true,"label":null}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    assert!(!diagnostics[0].primary.text.contains(".rs"));
}

// Unmappable diagnostics should preserve generated-code location.

#[test]
fn remapper_fallback_includes_generated_line_info() {
    // When all spans are unmappable, the tier-3 envelope should include
    // the generated .rs line/column in the primary label for debugging.
    let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/main.rs","line_start":40,"column_start":3,"line_end":40,"column_end":15,"is_primary":true,"label":"expected type"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    // The primary label should mention the generated-code location.
    assert!(
        diagnostics[0].primary.text.contains("line 40"),
        "fallback should mention generated line, got: {}",
        diagnostics[0].primary.text
    );
}

#[test]
fn remapper_individual_span_fallback_preserves_location() {
    // Even individual span fallbacks should include the generated location.
    let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":false,"label":"immutable borrow occurs here"},{"file_name":"src/main.rs","line_start":50,"column_start":8,"line_end":50,"column_end":20,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[]}"#;
    let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

    // The unmappable primary (line 50) should have location context.
    assert!(
        diagnostics[0].primary.text.contains("line 50"),
        "unmappable span should include generated line in label, got: {}",
        diagnostics[0].primary.text
    );
}
