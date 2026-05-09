use std::fs;

use kobo_driver::{run_check_pipeline, CompileSession, KoboConfig, KoboMode};
use kobo_errors::KErrorCode;

#[test]
fn check_pipeline_reports_multiple_parse_errors_when_recovery_enabled() {
    let root = std::env::temp_dir().join("kobo-driver-recovery-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("bad.kobo");
    fs::write(&file, "fn a() { let x = }\nfn b() { let y = }\n").unwrap();

    let mut config = KoboConfig::default();
    config.enable_parse_recovery = true;
    let mut session = CompileSession::new(config);

    let result = run_check_pipeline(&mut session, &file);
    assert!(result.is_err());
    assert!(
        session.diagnostics.len() >= 2,
        "expected multiple parse diagnostics, got {:?}",
        session.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_pipeline_continues_analysis_for_non_poisoned_function() {
    let root = std::env::temp_dir().join("kobo-driver-recovery-continuation-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("mixed.kobo");
    let source = r#"
fn broken() {
    let x =
}

fn good() {
    let config = String::from("hello");
    process(config);
    log(config);
}

fn process(s: String) {
    println!("process: {}", s);
}

fn log(s: String) {
    println!("log: {}", s);
}
"#;
    fs::write(&file, source).unwrap();

    let mut config = KoboConfig::default();
    config.mode = KoboMode::Checked;
    config.enable_parse_recovery = true;
    let mut session = CompileSession::new(config);

    let result = run_check_pipeline(&mut session, &file);
    assert!(
        result.is_err(),
        "parse diagnostics must still make the check fail"
    );
    assert!(
        session.diagnostics.iter().any(|diag| matches!(
            diag.code,
            KErrorCode::K0100 | KErrorCode::K0101 | KErrorCode::K0102
        )),
        "expected at least one parser recovery diagnostic"
    );
    assert!(
        session
            .diagnostics
            .iter()
            .any(|diag| diag.code == KErrorCode::K0001),
        "expected use-after-move analysis diagnostic from the non-poisoned good function"
    );

    let broken_start = source.find("fn broken").unwrap() as u32;
    let broken_end = source.find("fn good").unwrap() as u32;
    let cascades_in_poison = session
        .diagnostics
        .iter()
        .filter(|diag| {
            !matches!(
                diag.code,
                KErrorCode::K0100 | KErrorCode::K0101 | KErrorCode::K0102 | KErrorCode::K0103
            )
        })
        .filter(|diag| {
            diag.primary.span.start >= broken_start && diag.primary.span.end <= broken_end
        })
        .count();
    assert_eq!(
        cascades_in_poison, 0,
        "non-parser diagnostics must not point inside the poisoned function"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn poisoned_parse_regions_do_not_emit_duplicate_analysis_diagnostics() {
    let root = std::env::temp_dir().join("kobo-driver-cascade-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("bad.kobo");
    fs::write(
        &file,
        "fn a() { let moved = String::from(\"x\"); let y = ; println!(\"{}\", moved); }\n",
    )
    .unwrap();

    let mut config = KoboConfig::default();
    config.enable_parse_recovery = true;
    let mut session = CompileSession::new(config);

    let _ = run_check_pipeline(&mut session, &file);

    let parse_count = session
        .diagnostics
        .iter()
        .filter(|diag| diag.code.as_str().starts_with("K01") || diag.primary.text.contains("parse"))
        .count();
    assert!(parse_count >= 1);
    assert!(
        session.diagnostics.len() <= parse_count + 1,
        "poisoned syntax should not cascade into many unrelated diagnostics: {:?}",
        session.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}
