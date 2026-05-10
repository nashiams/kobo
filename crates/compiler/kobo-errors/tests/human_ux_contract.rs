use kobo_errors::{
    diagnostic_registry, explain_code, explain_code_with_detail, ExplainDetail, KErrorCode,
};

fn active_codes_to_check() -> Vec<KErrorCode> {
    vec![
        KErrorCode::K0001,
        KErrorCode::K0002,
        KErrorCode::K0025,
        KErrorCode::K0041,
        KErrorCode::K0042,
        KErrorCode::K0062,
        KErrorCode::K0063,
        KErrorCode::K0080,
        KErrorCode::K0100,
        KErrorCode::K0102,
        KErrorCode::K0107,
        KErrorCode::K0108,
    ]
}

#[test]
fn active_explain_pages_are_human_teaching_pages() {
    for code in active_codes_to_check() {
        let text = explain_code(code.as_str())
            .unwrap_or_else(|| panic!("missing explain text for {}", code.as_str()));

        assert!(
            text.contains("What happened") || text.contains("What Kobo found"),
            "{} explain should describe the problem in user language:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("How to fix") || text.contains("What to do"),
            "{} explain should include remediation:\n{text}",
            code.as_str()
        );
        assert!(
            !text.contains("machine edits:") && !text.contains("status: active"),
            "{} default explain should not dump registry metadata:\n{text}",
            code.as_str()
        );
    }
}

#[test]
fn verbose_explain_pages_keep_machine_metadata() {
    let text = explain_code_with_detail("K0107", ExplainDetail::Verbose)
        .expect("K0107 verbose explain should exist");

    assert!(text.contains("slug: unmodeled-external-boundary"));
    assert!(text.contains("status: active"));
    assert!(text.contains("machine edits:"));
}

#[test]
fn active_registry_text_has_no_mojibake() {
    let registry = diagnostic_registry();
    let bad = ["\u{00e2}", "\u{00c3}\u{00a2}", "\u{fffd}"];

    for code in active_codes_to_check() {
        let entry = registry
            .get(code)
            .unwrap_or_else(|| panic!("missing registry entry for {}", code.as_str()));
        let combined = format!(
            "{}\n{}\n{}\n{}",
            entry.title, entry.summary, entry.explain, entry.slug
        );

        for needle in bad {
            assert!(
                !combined.contains(needle),
                "{} registry text contains mojibake `{needle}`:\n{combined}",
                code.as_str()
            );
        }
    }
}
