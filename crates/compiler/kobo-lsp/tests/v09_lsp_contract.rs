use kobo_errors::{DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{FileSetBuilder, KoboSpan};

fn diagnostic(code: KErrorCode, source: &str, needle: &str) -> (kobo_ir::FileSet, KDiagnostic) {
    let mut files = FileSetBuilder::new();
    let file_id = files.add_file("src/main.kobo".into(), source.to_owned());
    let start = source.find(needle).expect("fixture needle must exist");
    let span = KoboSpan::new(start as u32, (start + needle.len()) as u32, file_id);
    let diagnostic = KDiagnostic::new(
        code,
        Severity::Error,
        DiagLabel::primary(span, "v0.9 LSP diagnostic"),
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
        "kobo replay",
    ] {
        assert!(
            text.contains(needle),
            "LSP payload should contain {needle}: {text}"
        );
    }
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
    for choice in ["model", "record", "stub", "outside", "opaque", "debt"] {
        assert!(
            text.contains(choice),
            "payload should expose boundary choice {choice}: {text}"
        );
    }
}
