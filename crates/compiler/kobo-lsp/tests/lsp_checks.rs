use kobo_errors::{CliSuggestion, DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{FileSetBuilder, KoboSpan};

fn diagnostic(code: KErrorCode, source: &str, needle: &str) -> (kobo_ir::FileSet, KDiagnostic) {
    let mut files = FileSetBuilder::new();
    let file_id = files.add_file("src/main.kobo".into(), source.to_owned());
    let start = source.find(needle).expect("fixture needle must exist");
    let span = KoboSpan::new(start as u32, (start + needle.len()) as u32, file_id);
    let diagnostic = KDiagnostic::new(
        code,
        Severity::Error,
        DiagLabel::primary(span, "runtime LSP diagnostic"),
        format!("diagnostic for {}", code.as_str()),
        DiagDecision("use the LSP action".to_owned()),
    );
    (files.finish(), diagnostic)
}

#[test]
fn k010x_payloads_include_registry_metadata_and_runnables() {
    let (file_set, diagnostic) = diagnostic(
        KErrorCode::K0100,
        "#[kobo::must_call(ack | nack)]\nstruct Delivery {}\n",
        "must_call",
    );

    let value = kobo_lsp::diagnostic_value(&file_set, &diagnostic, true)
        .expect("LSP diagnostic should serialize");
    let text = serde_json::to_string(&value).expect("value should serialize");

    for needle in [
        "K0100",
        "slug",
        "title",
        "severity",
        "range",
        "kobo explain K0100",
        "kobo test --sim quick",
    ] {
        assert!(
            text.contains(needle),
            "LSP payload should contain {needle}: {text}"
        );
    }
    assert!(
        !text.contains("kobo replay"),
        "LSP must not offer replay without a real witness artifact: {text}"
    );
}

#[test]
fn k0107_payload_exposes_grouped_boundary_choices() {
    let (file_set, diagnostic) = diagnostic(
        KErrorCode::K0107,
        "#[kobo::scenario(profile = \"async\")]\nfn main() { reqwest::Client::new(); }\n",
        "reqwest",
    );

    let value = kobo_lsp::diagnostic_value(&file_set, &diagnostic, true)
        .expect("LSP diagnostic should serialize");
    let text = serde_json::to_string(&value).expect("value should serialize");

    assert!(text.contains("K0107"), "payload should name K0107: {text}");
    assert!(
        text.contains("boundary-policy"),
        "payload should group boundary actions: {text}"
    );
    assert!(
        !text.contains("\"command\":null"),
        "K0107 boundary choices must not be inert code actions: {text}"
    );
    assert!(
        text.contains("kobo explain K0107 --verbose"),
        "K0107 boundary choices should route to the detailed policy explanation: {text}"
    );
    for choice in ["model", "record", "stub", "outside", "opaque", "debt"] {
        assert!(
            text.contains(choice),
            "payload should expose boundary choice {choice}: {text}"
        );
    }
}

#[test]
fn lsp_replay_action_uses_actual_witness_path_when_diagnostic_has_one() {
    let (file_set, diagnostic) = diagnostic(
        KErrorCode::K0100,
        "#[kobo::must_call(ack | nack)]\nstruct Delivery {}\n",
        "must_call",
    );
    let diagnostic = diagnostic.with_run(CliSuggestion(
        "kobo replay .kobo/witnesses/order-7.kwit".to_owned(),
    ));

    let value = kobo_lsp::diagnostic_value(&file_set, &diagnostic, true)
        .expect("LSP diagnostic should serialize");
    let text = serde_json::to_string(&value).expect("value should serialize");

    assert!(
        text.contains("kobo replay .kobo/witnesses/order-7.kwit"),
        "replay action must use the diagnostic witness path: {text}"
    );
    assert!(
        !text.contains("<witness>.kwit"),
        "actual witness diagnostics must not expose replay template placeholders: {text}"
    );
}
