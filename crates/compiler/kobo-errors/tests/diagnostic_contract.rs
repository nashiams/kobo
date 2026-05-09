use kobo_errors::{
    diagnostic_registry, resolve_severity, DiagnosticCategory, DiagnosticNote,
    DiagnosticRelatedInfo, DiagnosticStatus, DiagnosticSuggestion, KErrorCode, MachineEditPolicy,
    ModeBehavior, Severity, SuggestionApplicability, SuggestionPolicy, TextEdit,
};
use kobo_ir::{FileId, FileSetBuilder, KoboMode, KoboSpan};

#[test]
fn registry_has_metadata_for_active_codes() {
    let registry = diagnostic_registry();
    for code in [
        KErrorCode::K0001,
        KErrorCode::K0002,
        KErrorCode::K0025,
        KErrorCode::K0061,
        KErrorCode::K0080,
        KErrorCode::K0099,
    ] {
        let entry = registry.get(code).expect("active code must be registered");
        assert_eq!(entry.code, code);
        assert!(!entry.title.trim().is_empty());
        assert!(!entry.summary.trim().is_empty());
        assert!(!entry.explain.trim().is_empty());
    }
}

#[test]
fn registry_contains_public_slug_and_policy_fields() {
    let registry = diagnostic_registry();
    for code in [
        KErrorCode::K0001,
        KErrorCode::K0061,
        KErrorCode::K0099,
        KErrorCode::K0100,
        KErrorCode::K0101,
        KErrorCode::K0102,
        KErrorCode::K0103,
        KErrorCode::K0104,
        KErrorCode::K0105,
        KErrorCode::K0107,
        KErrorCode::K0108,
    ] {
        let entry = registry.get(code).expect("active code must be registered");
        assert!(!entry.slug.trim().is_empty(), "{code} needs public slug");
        assert_ne!(
            entry.slug, entry.code_text,
            "{code} slug cannot be the code"
        );
        assert!(
            entry
                .slug
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
            "{code} slug must be lowercase kebab case: {}",
            entry.slug
        );
        assert!(!entry.title.trim().is_empty(), "{code} needs title");
        assert!(!entry.summary.trim().is_empty(), "{code} needs summary");
        assert!(
            !entry.explain.trim().is_empty(),
            "{code} needs explain text"
        );
        assert_ne!(
            entry.category,
            DiagnosticCategory::Parser,
            "{code} is not parser recovery"
        );
        assert_ne!(
            entry.mode_behavior,
            ModeBehavior::Unspecified,
            "{code} needs mode behavior"
        );
        assert_ne!(
            entry.suggestion_policy,
            SuggestionPolicy::Unspecified,
            "{code} needs suggestion policy"
        );
        assert_ne!(
            entry.machine_edit_policy,
            MachineEditPolicy::Unspecified,
            "{code} needs machine-edit policy"
        );
    }
}

#[test]
fn source_coverage_k010x_codes_match_registry() {
    let registry = diagnostic_registry();
    let expectations = [
        (
            KErrorCode::K0100,
            "liveness-obligation-unresolved",
            DiagnosticCategory::Liveness,
            "commit",
        ),
        (
            KErrorCode::K0101,
            "liveness-obligation-escaped",
            DiagnosticCategory::Liveness,
            "escape",
        ),
        (
            KErrorCode::K0102,
            "raw-nondeterminism-in-scenario",
            DiagnosticCategory::Nondeterminism,
            "nondeterminism",
        ),
        (
            KErrorCode::K0103,
            "malformed-must-call-attribute",
            DiagnosticCategory::Liveness,
            "must_call",
        ),
        (
            KErrorCode::K0104,
            "invalid-kwit-witness-schema",
            DiagnosticCategory::BoundaryPolicy,
            "schema_version 0",
        ),
        (
            KErrorCode::K0105,
            "malformed-scenario-metadata",
            DiagnosticCategory::Nondeterminism,
            "scenario",
        ),
        (
            KErrorCode::K0107,
            "unmodeled-external-boundary",
            DiagnosticCategory::BoundaryPolicy,
            "normal Rust crates remain allowed",
        ),
        (
            KErrorCode::K0108,
            "replay-obligation-suppressed",
            DiagnosticCategory::BoundaryPolicy,
            "reviewable evidence",
        ),
    ];

    for (code, slug, category, wording) in expectations {
        let entry = registry.get(code).expect("v0.8.5 K010x code must exist");
        assert_eq!(entry.slug, slug);
        assert_eq!(entry.category, category);
        assert!(
            entry.summary.contains(wording) || entry.explain.contains(wording),
            "{code} registry text must contain `{wording}`"
        );
    }

    for parser_code in [
        KErrorCode::K0110,
        KErrorCode::K0111,
        KErrorCode::K0112,
        KErrorCode::K0113,
    ] {
        let entry = registry
            .get(parser_code)
            .expect("parser recovery code must be registered");
        assert_eq!(entry.category, DiagnosticCategory::Parser);
        assert!(entry.code_text.starts_with("K011"));
    }
}

#[test]
fn every_emitted_active_code_is_registered_and_marked_active() {
    let registry = diagnostic_registry();
    let active_codes = [
        KErrorCode::K0001,
        KErrorCode::K0002,
        KErrorCode::K0020,
        KErrorCode::K0021,
        KErrorCode::K0025,
        KErrorCode::K0026,
        KErrorCode::K0030,
        KErrorCode::K0031,
        KErrorCode::K0032,
        KErrorCode::K0041,
        KErrorCode::K0042,
        KErrorCode::K0043,
        KErrorCode::K0044,
        KErrorCode::K0060,
        KErrorCode::K0061,
        KErrorCode::K0062,
        KErrorCode::K0063,
        KErrorCode::K0064,
        KErrorCode::K0065,
        KErrorCode::K0067,
        KErrorCode::K0019,
        KErrorCode::K0080,
        KErrorCode::K0080P1,
        KErrorCode::K0080P2,
        KErrorCode::K0080P3,
        KErrorCode::K0080P4,
        KErrorCode::K0081,
        KErrorCode::K0082,
        KErrorCode::K0083,
        KErrorCode::K0084,
        KErrorCode::K0085,
        KErrorCode::K0090,
        KErrorCode::K0095,
        KErrorCode::K0099,
        KErrorCode::K0100,
        KErrorCode::K0101,
        KErrorCode::K0102,
        KErrorCode::K0103,
        KErrorCode::K0104,
        KErrorCode::K0105,
        KErrorCode::K0107,
        KErrorCode::K0108,
        KErrorCode::K0110,
        KErrorCode::K0111,
        KErrorCode::K0112,
        KErrorCode::K0113,
        KErrorCode::K0107,
        KErrorCode::K0108,
    ];

    for code in active_codes {
        let entry = registry
            .get(code)
            .expect("emitted active code must be registered");
        assert_eq!(
            entry.status,
            DiagnosticStatus::Active,
            "{code} must be active"
        );
        assert_eq!(entry.code_text, code.as_str());
        assert_eq!(code.metadata().short_description, entry.title);
    }
}

#[test]
fn registry_drives_mode_dependent_severity() {
    assert_eq!(resolve_severity(KErrorCode::K0001, KoboMode::Script), None);
    assert_eq!(
        resolve_severity(KErrorCode::K0001, KoboMode::Checked),
        Some(Severity::Warning)
    );
    assert_eq!(
        resolve_severity(KErrorCode::K0001, KoboMode::Strict),
        Some(Severity::Error)
    );
    assert_eq!(
        resolve_severity(KErrorCode::K0100, KoboMode::Script),
        Some(Severity::Warning)
    );
    assert_eq!(
        resolve_severity(KErrorCode::K0100, KoboMode::Checked),
        Some(Severity::Warning)
    );
    assert_eq!(
        resolve_severity(KErrorCode::K0100, KoboMode::Strict),
        Some(Severity::Error)
    );
}

#[test]
fn registry_reserves_parser_recovery_codes() {
    let registry = diagnostic_registry();
    for code_text in ["K0110", "K0111", "K0112", "K0113"] {
        let entry = registry
            .find_by_code_text(code_text)
            .expect("parser recovery code must be registered");
        assert_eq!(entry.default_severity, Severity::Error);
        assert!(
            entry.summary.contains("syntax")
                || entry.summary.contains("delimiter")
                || entry.summary.contains("item")
                || entry.summary.contains("recovery")
        );
    }
}

#[test]
fn registry_reserves_boundary_and_replay_codes() {
    let registry = diagnostic_registry();
    for code_text in ["K0107", "K0108"] {
        let entry = registry
            .find_by_code_text(code_text)
            .expect("v0.8.5 boundary/replay code must be registered");
        assert!(matches!(
            entry.default_severity,
            Severity::Warning | Severity::Error
        ));
        assert!(entry.summary.contains("boundary") || entry.summary.contains("replay"));
    }
}

#[test]
fn diagnostic_carries_related_info_notes_and_machine_suggestions() {
    let diagnostic = kobo_errors::KDiagnostic::new(
        KErrorCode::K0001,
        Severity::Error,
        kobo_errors::DiagLabel::primary(KoboSpan::new(10, 14, FileId(0)), "used here"),
        "value was moved",
        "clone or restructure ownership",
    )
    .with_note(DiagnosticNote::new("move happened on another path"))
    .with_related_info(DiagnosticRelatedInfo::new(
        KoboSpan::new(1, 6, FileId(0)),
        "move happened here",
    ))
    .with_suggestion(DiagnosticSuggestion::new(
        "clone the value before this use",
        SuggestionApplicability::MaybeIncorrect,
        vec![TextEdit::replace(
            KoboSpan::new(10, 14, FileId(0)),
            "name.clone()",
        )],
    ));

    assert_eq!(diagnostic.notes.len(), 1);
    assert_eq!(diagnostic.related.len(), 1);
    assert_eq!(diagnostic.suggestions.len(), 1);
    assert_eq!(
        diagnostic.suggestions[0].edits[0].replacement,
        "name.clone()"
    );
}

#[test]
fn json_diagnostic_contains_code_severity_spans_and_suggestions() {
    let mut files = FileSetBuilder::new();
    files.add_file("demo.kobo".into(), "abcdef\n".to_owned());

    let diagnostic = kobo_errors::KDiagnostic::new(
        KErrorCode::K0099,
        Severity::Error,
        kobo_errors::DiagLabel::primary(KoboSpan::new(1, 5, FileId(0)), "rustc error"),
        "generated Rust failed",
        "inspect source map",
    )
    .with_suggestion(DiagnosticSuggestion::new(
        "replace generated fragment",
        SuggestionApplicability::MachineApplicable,
        vec![TextEdit::replace(KoboSpan::new(1, 5, FileId(0)), "fixed")],
    ));

    let value = kobo_errors::diagnostic_to_json_value(files.as_file_set(), &diagnostic);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["code"], "K0099");
    assert_eq!(value["severity"], "error");
    assert_eq!(value["primary"]["file_path"], "demo.kobo");
    assert_eq!(value["primary"]["byte_start"], 1);
    assert_eq!(value["primary"]["byte_end"], 5);
    assert_eq!(value["primary"]["line_start"], 0);
    assert_eq!(value["primary"]["column_start"], 1);
    assert_eq!(value["suggestions"][0]["edits"][0]["replacement"], "fixed");
    assert_eq!(value["suggestions"][0]["edits"][0]["byte_start"], 1);
    assert_eq!(
        value["suggestions"][0]["applicability"],
        "machine-applicable"
    );
}

#[test]
fn typed_json_and_lsp_payloads_share_the_same_diagnostic_data() {
    let mut files = FileSetBuilder::new();
    files.add_file("demo.kobo".into(), "fn main() { let x = ; }\n".to_owned());

    let diagnostic = kobo_errors::KDiagnostic::new(
        KErrorCode::K0110,
        Severity::Error,
        kobo_errors::DiagLabel::primary(KoboSpan::new(10, 16, FileId(0)), "syntax error"),
        "expected expression",
        "skip poisoned region and continue parsing",
    )
    .with_related_info(DiagnosticRelatedInfo::new(
        KoboSpan::new(0, 9, FileId(0)),
        "function started here",
    ))
    .with_suggestion(DiagnosticSuggestion::new(
        "insert a placeholder expression",
        SuggestionApplicability::HasPlaceholders,
        vec![TextEdit::replace(
            KoboSpan::new(14, 14, FileId(0)),
            "todo!()",
        )],
    ));

    let file_set = files.as_file_set();
    let json = kobo_errors::DiagnosticJson::from_diagnostic(file_set, &diagnostic);
    assert_eq!(json.schema_version, 1);
    assert_eq!(json.code, "K0110");
    assert_eq!(json.slug, "syntax-error-recovered");
    assert_eq!(json.title, "syntax error recovered");
    assert_eq!(json.primary.file_path.as_deref(), Some("demo.kobo"));
    assert_eq!(json.primary.byte_start, 10);
    assert_eq!(json.primary.byte_end, 16);
    assert_eq!(json.primary.line_start, 0);
    assert_eq!(json.primary.column_start, 10);
    assert_eq!(json.suggestions[0].applicability, "has-placeholders");
    assert_eq!(json.suggestions[0].edits[0].replacement, "todo!()");

    let lsp = kobo_errors::DiagnosticLspPayload::from_diagnostic(file_set, &diagnostic);
    assert_eq!(lsp.source, "kobo");
    assert_eq!(lsp.uri.as_deref(), Some("demo.kobo"));
    assert_eq!(lsp.code.as_deref(), Some("K0110"));
    assert_eq!(lsp.range.start.line, 0);
    assert_eq!(lsp.range.start.character, 10);
    assert!(lsp
        .code_description
        .as_deref()
        .is_some_and(|value| value.contains("K0110")));
    assert_eq!(lsp.related_information.len(), 1);
    assert_eq!(
        lsp.data["suggestions"][0]["edits"][0]["replacement"],
        "todo!()"
    );
}

#[test]
fn no_color_render_contains_plain_default_card() {
    let mut files = FileSetBuilder::new();
    files.add_file("demo.kobo".into(), "fn main() { let x = ; }\n".to_owned());
    let diagnostic = kobo_errors::KDiagnostic::new(
        KErrorCode::K0110,
        Severity::Error,
        kobo_errors::DiagLabel::primary(KoboSpan::new(18, 19, FileId(0)), "expected expression"),
        "expected expression",
        "skip poisoned region and continue parsing",
    );

    let rendered = kobo_errors::render_diagnostic_card(
        files.as_file_set(),
        &diagnostic,
        kobo_errors::ColorMode::Never,
    );
    assert!(rendered.contains("error[K0110]"));
    assert!(rendered.contains("expected expression"));
    assert!(rendered.contains("--> demo.kobo:1:19"));
    assert!(!rendered.contains("\u{1b}["));
}

#[test]
fn render_preserves_related_notes_and_suggestions_in_no_color_mode() {
    let mut files = FileSetBuilder::new();
    files.add_file(
        "demo.kobo".into(),
        "fn main() {\n    use_name(name);\n}\n".to_owned(),
    );
    let diagnostic = kobo_errors::KDiagnostic::new(
        KErrorCode::K0001,
        Severity::Warning,
        kobo_errors::DiagLabel::primary(KoboSpan::new(25, 29, FileId(0)), "used here"),
        "value was moved",
        "clone or restructure ownership",
    )
    .with_note(DiagnosticNote::new("the move came from a prior branch"))
    .with_related_info(DiagnosticRelatedInfo::new(
        KoboSpan::new(5, 9, FileId(0)),
        "value moved here",
    ))
    .with_suggestion(DiagnosticSuggestion::new(
        "clone before the later use",
        SuggestionApplicability::MaybeIncorrect,
        vec![TextEdit::replace(
            KoboSpan::new(25, 29, FileId(0)),
            "name.clone()",
        )],
    ));

    let rendered = kobo_errors::render_diagnostic_card(
        files.as_file_set(),
        &diagnostic,
        kobo_errors::ColorMode::Never,
    );

    assert!(rendered.contains("warning[K0001]"));
    assert!(rendered.contains("--> demo.kobo:2:14"));
    assert!(rendered.contains("the move came from a prior branch"));
    assert!(rendered.contains("value moved here"));
    assert!(rendered.contains("clone before the later use"));
    assert!(rendered.contains("name.clone()"));
    assert!(!rendered.contains("\u{1b}["));
}
