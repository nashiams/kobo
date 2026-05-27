mod cli_common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cli_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo_with_timeout, s,
    CliOutput, TestProject,
};
use serde_json::Value;

const V12_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V12_TIMEOUT)
}

fn write_rust_only_project(label: &str) -> TestProject {
    let project = TestProject::new(label);
    project.write(
        "Cargo.toml",
        r#"[package]
name = "rust_only_service"
version = "0.1.0"
edition = "2021"

[dependencies]
rand = "0.8"
reqwest = "0.12"
"#,
    );
    project.write(
        "src/main.rs",
        r#"
use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

fn main() {
    let state = Rc::new(RefCell::new(0_u64));
    let worker = std::thread::spawn(|| {
        let _work = 1 + 1;
    });
    let _now = SystemTime::now();
    let _random = rand::random::<u64>();
    let _request = reqwest::get("https://example.test/service");
    println!("{}", state.borrow());
}
"#,
    );
    project
}

fn debt_json(project: &TestProject) -> Value {
    let output = run_kobo(
        &[
            s("debt"),
            s("--cargo"),
            path_arg(&project.root),
            s("--json"),
        ],
        &project.root,
    );
    assert_success(&output, "standalone cargo debt JSON should succeed");
    serde_json::from_str(&output.stdout).expect("debt --cargo --json should be JSON")
}

#[test]
fn debt_scans_rust_only_cargo_project() {
    let project = write_rust_only_project("standalone-debt-scan");
    assert!(
        project.find_files_with_ext("kobo").is_empty(),
        "fixture must not contain Kobo source"
    );

    let output = run_kobo(
        &[s("debt"), s("--cargo"), path_arg(&project.root)],
        &project.root,
    );

    assert_success(&output, "debt --cargo should scan a Rust-only project");
    let text = output.combined();
    assert_contains(
        &text,
        "Standalone Rust debt",
        "human output should name the mode",
    );
    assert_contains(
        &text,
        "src/main.rs",
        "human output should include Rust source file paths",
    );
    assert_contains(
        &text,
        "advisory",
        "standalone debt must disclose advisory precision",
    );
}

#[test]
fn debt_reports_liveness_candidate_with_source_span() {
    let project = write_rust_only_project("standalone-debt-liveness");
    let json = debt_json(&project);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");
    let finding = findings
        .iter()
        .find(|finding| finding["kind"] == "liveness-candidate")
        .expect("thread handle should produce a liveness candidate");

    assert_eq!(finding["symbol"], "worker");
    assert_eq!(finding["severity"], "advisory");
    assert_eq!(finding["blocking"], false);
    assert!(
        finding["line"].as_u64().is_some_and(|line| line > 0),
        "liveness finding should include a source line: {finding}"
    );
    assert!(
        finding["span"]["start"].as_u64().is_some(),
        "liveness finding should include byte span evidence: {finding}"
    );
}

#[test]
fn debt_reports_multiline_spawn_liveness_candidate() {
    let project = TestProject::new("standalone-debt-multiline-spawn");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "multiline_spawn"
version = "0.1.0"
edition = "2021"
"#,
    );
    project.write(
        "src/main.rs",
        r#"
fn main() {
    let worker =
        std::thread::spawn(
        || {
            println!("background");
        },
    );
    let _ = worker.thread().id();
}
"#,
    );

    let json = debt_json(&project);
    let findings = json["findings"]
        .as_array()
        .expect("findings should be array");
    let finding = findings
        .iter()
        .find(|finding| finding["kind"] == "liveness-candidate")
        .expect("multi-line thread spawn should produce a liveness candidate");

    assert_eq!(finding["symbol"], "worker");
    assert_contains(
        &finding["evidence"].to_string(),
        "std::thread::spawn",
        "multi-line liveness finding should preserve spawn evidence",
    );
}

#[test]
fn debt_reports_aliased_spawn_and_dependency_calls() {
    let project = TestProject::new("standalone-debt-aliases");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "alias_debt"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = "0.12"
"#,
    );
    project.write(
        "src/main.rs",
        r#"
use std::thread::spawn as thread_spawn;
use reqwest::get as http_get;

fn main() {
    let worker = thread_spawn(|| {
        println!("background");
    });
    let _request = http_get("https://example.test/service");
    observe_worker(worker);
}

fn observe_worker(worker: std::thread::JoinHandle<()>) {
    worker.join().expect("worker should join");
}
"#,
    );

    let json = debt_json(&project);
    let text = json["findings"].to_string();
    assert_contains(
        &text,
        "thread_spawn",
        "standalone debt should recognize aliased thread spawn calls",
    );
    assert_contains(
        &text,
        "http_get",
        "standalone debt should recognize aliased external dependency calls",
    );
    assert_contains(
        &text,
        "observed-by-helper",
        "standalone debt should note helper-observed handles instead of treating them as invisible",
    );
}

#[test]
fn debt_reports_nondeterminism_boundary_candidate() {
    let project = write_rust_only_project("standalone-debt-nondeterminism");
    let json = debt_json(&project);
    let text = json["findings"].to_string();

    for expected in ["SystemTime::now", "rand::random", "reqwest"] {
        assert_contains(
            &text,
            expected,
            "nondeterminism and external boundary candidates should be reported",
        );
    }
}

#[test]
fn debt_json_includes_cargo_dependency_context() {
    let project = write_rust_only_project("standalone-debt-json-cargo");
    let json = debt_json(&project);

    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["mode"], "rust-cargo-standalone");
    assert_eq!(json["precision"], "advisory");
    assert_eq!(json["cargo"]["package"], "rust_only_service");
    let deps = json["cargo"]["dependencies"]
        .as_array()
        .expect("dependencies should be an array")
        .iter()
        .filter_map(|dep| dep.as_str())
        .collect::<Vec<_>>();
    assert!(
        deps.contains(&"rand"),
        "dependencies should include rand: {json}"
    );
    assert!(
        deps.contains(&"reqwest"),
        "dependencies should include reqwest: {json}"
    );
}

#[test]
fn debt_does_not_require_kobo_source_file() {
    let project = write_rust_only_project("standalone-debt-no-kobo-source");
    let output = run_kobo(
        &[
            s("debt"),
            s("--cargo"),
            path_arg(&project.root),
            s("--json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "debt --cargo should not require a .kobo source file",
    );
    assert_not_contains(
        &output.combined(),
        "failed to build KIR",
        "standalone debt must not route Rust-only projects through Kobo KIR",
    );
}

#[test]
fn debt_distinguishes_advisory_from_blocking_diagnostic() {
    let project = write_rust_only_project("standalone-debt-advisory");
    let json = debt_json(&project);
    assert_eq!(json["blocking"], false);
    assert!(
        json["findings"]
            .as_array()
            .expect("findings should be an array")
            .iter()
            .all(|finding| finding["severity"] == "advisory" && finding["blocking"] == false),
        "all standalone Rust findings should be advisory: {json}"
    );

    let cargo_metadata = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(&project.root)
        .output()
        .expect("cargo metadata should launch");
    assert!(
        cargo_metadata.status.success(),
        "advisory Kobo debt must not block normal Cargo metadata usage\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cargo_metadata.stdout),
        String::from_utf8_lossy(&cargo_metadata.stderr)
    );
}
