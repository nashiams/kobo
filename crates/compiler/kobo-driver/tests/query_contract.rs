use std::fs;

use kobo_driver::{KoboConfig, KoboMode, QuerySession};
use kobo_errors::KErrorCode;

#[test]
fn parse_query_is_cached_per_source_fingerprint() {
    let root = std::env::temp_dir().join("kobo-query-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("demo.kobo");
    fs::write(&file, "fn main() { println!(\"hi\"); }\n").unwrap();

    let mut query = QuerySession::new(KoboConfig::default());
    let first = query.parse(&file).expect("first parse should succeed");
    let second = query.parse(&file).expect("second parse should succeed");

    assert_eq!(first.file_id, second.file_id);
    assert_eq!(query.metrics().parse_executions, 1);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repeated_phase_queries_reuse_cached_results() {
    let root = std::env::temp_dir().join("kobo-query-phase-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("demo.kobo");
    fs::write(
        &file,
        "fn main() { let name = String::from(\"Ada\"); println!(\"{}\", name); }\n",
    )
    .unwrap();

    let mut query = QuerySession::new(KoboConfig::default());
    query.kir(&file).expect("kir should build");
    query
        .diagnostics(&file)
        .expect("diagnostics should compute");
    query.codegen(&file).expect("codegen should compute");
    query.kir(&file).expect("kir should be cached");
    query
        .diagnostics(&file)
        .expect("diagnostics should be cached");
    query.codegen(&file).expect("codegen should be cached");

    let metrics = query.metrics();
    assert_eq!(metrics.parse_executions, 1);
    assert_eq!(metrics.kir_executions, 1);
    assert_eq!(metrics.analysis_executions, 1);
    assert_eq!(metrics.codegen_executions, 1);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_content_change_invalidates_downstream_queries() {
    let root = std::env::temp_dir().join("kobo-query-source-invalidation-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("demo.kobo");
    fs::write(&file, "fn main() { println!(\"one\"); }\n").unwrap();

    let mut query = QuerySession::new(KoboConfig::default());
    let first = query.codegen(&file).expect("first codegen should compute");
    fs::write(&file, "fn main() { println!(\"two\"); }\n").unwrap();
    let second = query
        .codegen(&file)
        .expect("second codegen should compute after source change");

    assert_ne!(first.artifacts.rs_source, second.artifacts.rs_source);
    let metrics = query.metrics();
    assert_eq!(metrics.parse_executions, 2);
    assert_eq!(metrics.kir_executions, 2);
    assert_eq!(metrics.codegen_executions, 2);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn relevant_config_change_invalidates_affected_downstream_queries_without_reparsing() {
    let root = std::env::temp_dir().join("kobo-query-config-invalidation-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("demo.kobo");
    fs::write(
        &file,
        "fn main() {\n    let config = String::from(\"hello\");\n    process(config);\n    log(config);\n}\nfn process(s: String) { println!(\"{}\", s); }\nfn log(s: String) { println!(\"{}\", s); }\n",
    )
    .unwrap();

    let mut query = QuerySession::new(KoboConfig::default());
    let script_diagnostics = query
        .diagnostics(&file)
        .expect("script diagnostics should compute");

    let mut checked = KoboConfig::default();
    checked.mode = KoboMode::Checked;
    query.set_config(checked);
    let checked_diagnostics = query
        .diagnostics(&file)
        .expect("checked diagnostics should compute");

    assert!(script_diagnostics
        .visible
        .iter()
        .all(|diag| diag.code != KErrorCode::K0001));
    assert!(checked_diagnostics
        .visible
        .iter()
        .any(|diag| diag.code == KErrorCode::K0001));
    let metrics = query.metrics();
    assert_eq!(
        metrics.parse_executions, 1,
        "mode-only config changes must reuse parse output"
    );
    assert_eq!(
        metrics.analysis_executions, 2,
        "mode-dependent diagnostics must recompute"
    );
    assert_eq!(metrics.diagnostics_executions, 2);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn old_pipeline_api_and_query_api_produce_same_generated_source() {
    let root = std::env::temp_dir().join("kobo-query-compat-contract");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("demo.kobo");
    fs::write(&file, "fn main() { println!(\"same\"); }\n").unwrap();

    let mut session = kobo_driver::CompileSession::new(KoboConfig::default());
    let old_source = kobo_driver::run_pipeline(&mut session, &file).expect("old API should work");

    let mut query = QuerySession::new(KoboConfig::default());
    let new_source = query
        .codegen(&file)
        .expect("query API should work")
        .artifacts
        .rs_source
        .clone();

    assert_eq!(old_source, new_source);

    let _ = fs::remove_dir_all(root);
}
