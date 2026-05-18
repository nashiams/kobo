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

fn all_registry_entries() -> Vec<DiagnosticRegistryEntry> {
    let registry = diagnostic_registry();
    KErrorCode::ALL
        .iter()
        .map(|code| {
            registry
                .get(*code)
                .unwrap_or_else(|| panic!("missing registry entry for {}", code.as_str()))
                .clone()
        })
        .collect()
}

#[test]
fn every_enum_code_is_registered() {
    let registry = diagnostic_registry();

    for code in KErrorCode::ALL {
        assert!(
            registry.find_by_code_text(code.as_str()).is_some(),
            "{} exists in KErrorCode but has no registry entry",
            code.as_str()
        );
    }
}

#[test]
fn every_k_code_has_elm_level_explain_page() {
    for code in KErrorCode::ALL {
        let text = explain_code(code.as_str())
            .unwrap_or_else(|| panic!("missing explain text for {}", code.as_str()));

        assert!(
            text.contains("What happened"),
            "{} explain must say what happened:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("Why this matters"),
            "{} explain must teach why it matters:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("How to fix"),
            "{} explain must include fix guidance:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("Option 1:"),
            "{} explain must offer explicit fix choices:\n{text}",
            code.as_str()
        );
        assert!(
            text.contains("Example") && text.contains("Problem:") && text.contains("Fix:"),
            "{} explain must include a concrete problem/fix example:\n{text}",
            code.as_str()
        );
    }
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
fn all_registry_text_has_no_mojibake() {
    let bad = ["\u{00e2}", "\u{00c3}\u{00a2}", "\u{fffd}"];

    for entry in all_registry_entries() {
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
fn active_codes_do_not_use_explain_fallbacks() {
    let fallback_prose = [
        "Review the highlighted source and make the ownership or boundary choice explicit.",
        "Follow the help text from the diagnostic card and rerun Kobo.",
        "Apply a machine-applicable suggestion only after checking that it preserves the source intent.",
        "Choose an explicit boundary policy so the guarantee remains reviewable.",
    ];

    for entry in active_registry_entries() {
        let text = explain_code(entry.code_text)
            .unwrap_or_else(|| panic!("missing explain text for {}", entry.code_text));
        for fallback in fallback_prose {
            assert!(
                !text.contains(fallback),
                "{} active explain page used fallback prose `{fallback}`:\n{text}",
                entry.code_text
            );
        }
    }
}

#[test]
fn v011_active_explain_pages_have_specific_prod_depth_guidance() {
    let fallback_prose = [
        "Review the highlighted source and make the ownership or boundary choice explicit.",
        "Follow the help text from the diagnostic card and rerun Kobo.",
        "Apply a machine-applicable suggestion only after checking that it preserves the source intent.",
        "Choose an explicit boundary policy so the guarantee remains reviewable.",
    ];

    for code_text in [
        "K0120", "K0121", "K0122", "K0123", "K0124", "K0125", "K0126", "K0127", "K0128", "K0129",
    ] {
        let text = explain_code(code_text)
            .unwrap_or_else(|| panic!("missing explain text for {code_text}"));
        assert!(
            text.contains("Option 1:") && text.contains("Problem:") && text.contains("Fix:"),
            "{code_text} explain must be a concrete teaching page:\n{text}"
        );
        for fallback in fallback_prose {
            assert!(
                !text.contains(fallback),
                "{code_text} explain used generic fallback prose `{fallback}`:\n{text}"
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

    for code in KErrorCode::ALL {
        let text = explain_code(code.as_str()).expect("explain page should exist");
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "{} default explain leaked `{needle}`:\n{text}",
                code.as_str()
            );
        }
    }
}
