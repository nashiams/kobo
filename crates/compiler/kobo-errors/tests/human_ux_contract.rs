use kobo_errors::{
    diagnostic_registry, explain_code, explain_code_with_detail, DiagnosticRegistryEntry,
    ExplainDetail, KErrorCode,
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

fn active_registry_entries() -> Vec<DiagnosticRegistryEntry> {
    diagnostic_registry().active_entries().cloned().collect()
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
            text.contains("Example"),
            "{} explain should include a small before/after example:\n{text}",
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
    let bad = ["\u{00e2}", "\u{00c3}\u{00a2}", "\u{fffd}"];

    for entry in active_registry_entries() {
        let combined = format!(
            "{}\n{}\n{}\n{}",
            entry.title, entry.summary, entry.explain, entry.slug
        );

        for needle in bad {
            assert!(
                !combined.contains(needle),
                "{} registry text contains mojibake `{needle}`:\n{combined}",
                entry.code_text
            );
        }
    }
}

#[test]
fn every_active_explain_page_has_specific_fix_and_example() {
    let generic_fallbacks = [
        "Review the highlighted source and make the ownership or boundary choice explicit.",
        "Follow the help text from the diagnostic card and rerun Kobo.",
        "Apply a machine-applicable suggestion only after checking that it preserves the source intent.",
    ];

    for entry in active_registry_entries() {
        let text = explain_code(entry.code_text)
            .unwrap_or_else(|| panic!("missing explain text for {}", entry.code_text));

        assert!(
            text.contains("What happened"),
            "{} explain must say what happened:\n{text}",
            entry.code_text
        );
        assert!(
            text.contains("Why this matters"),
            "{} explain must teach why it matters:\n{text}",
            entry.code_text
        );
        assert!(
            text.contains("How to fix"),
            "{} explain must include a fix section:\n{text}",
            entry.code_text
        );
        assert!(
            text.contains("Option 1:"),
            "{} explain must give explicit fix choices:\n{text}",
            entry.code_text
        );
        assert!(
            text.contains("Example") && text.contains("Problem:") && text.contains("Fix:"),
            "{} explain must include a concrete problem/fix example:\n{text}",
            entry.code_text
        );
        for fallback in generic_fallbacks {
            assert!(
                !text.contains(fallback),
                "{} explain used generic fallback prose `{fallback}`:\n{text}",
                entry.code_text
            );
        }
    }
}

#[test]
fn high_traffic_explain_pages_include_small_examples() {
    for code in active_codes_to_check() {
        let text = explain_code(code.as_str()).expect("explain page should exist");
        assert!(
            text.contains("Example"),
            "{} explain should include a small example:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("Problem") || text.contains("Before"),
            "{} explain should show the bad shape:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("Fix") || text.contains("After"),
            "{} explain should show a fix shape:\n{text}",
            code.as_str()
        );
    }
}

#[test]
fn high_traffic_explain_pages_use_option_style_fix_guidance() {
    for code in active_codes_to_check() {
        let text = explain_code(code.as_str()).expect("explain page should exist");
        assert!(
            text.contains("Option 1:"),
            "{} explain should give fix guidance as explicit choices:\n{text}",
            code.as_str()
        );
    }
}

#[test]
fn default_explain_pages_avoid_internal_solver_jargon() {
    let forbidden = [
        "reserved diagnostic code",
        "greedy",
        "engine ceiling",
        "constraint cluster",
        "constraint conflict",
        "constraint graph",
        "solver input",
        "solver",
        "poisoned region",
        "poisoned",
        "boundary guards",
        "async-aware guard protocol",
        "internal solver",
        "machine edits:",
    ];

    for entry in active_registry_entries() {
        let text = explain_code(entry.code_text).expect("explain page should exist");
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "{} default explain leaked `{needle}`:\n{text}",
                entry.code_text
            );
        }
    }
}
