use std::fs;
use std::path::{Path, PathBuf};

use kobo_driver::{KoboConfig, QuerySession};

#[test]
fn solver_and_codegen_queries_change_when_compiler_shaped_source_changes() {
    let root = temp_root("kobo-self-host-query-source");
    let file = root.join("registry.kobo");
    fs::write(
        &file,
        r#"
pub fn lookup(code: &str) -> &'static str {
    if code == "K0107" { "boundary" } else { "unknown" }
}
"#,
    )
    .unwrap();

    let mut query = QuerySession::new(KoboConfig::default());
    let first = query.codegen(&file).expect("first codegen should succeed");
    let first_source = first.artifacts.rs_source.clone();

    fs::write(
        &file,
        r#"
pub fn lookup(code: &str) -> &'static str {
    if code == "K0107" { "unmodeled-external-boundary" } else { "unknown" }
}
"#,
    )
    .unwrap();

    let second = query.codegen(&file).expect("second codegen should succeed");
    assert_ne!(first_source, second.artifacts.rs_source);
    assert!(query.metrics().parse_executions >= 2);
    assert!(query.metrics().kir_executions >= 2);
    assert!(query.metrics().codegen_executions >= 2);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn query_source_does_not_use_temporary_solver_fingerprint() {
    let query_source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/query.rs"))
            .expect("query source should read");
    assert!(
        !query_source.contains("\"current-kir\""),
        "QuerySession solver keys must use a real KIR or constraint fingerprint"
    );
    assert!(
        !query_source.contains("rewritten_hash: source_hash"),
        "QuerySession preprocess keys must fingerprint rewritten source, not copy source_hash"
    );
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
