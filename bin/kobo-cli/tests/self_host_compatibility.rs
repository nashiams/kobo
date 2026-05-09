use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct CmdOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

struct CanaryCase {
    root: PathBuf,
}

impl CanaryCase {
    fn new() -> Self {
        let workspace = workspace_root();
        let unique = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace
            .join("target-test-fixtures")
            .join(format!("self-host-canary-{}-{unique}", std::process::id()));
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        copy_dir_all(&workspace.join("tests/fixtures/self_host_canary"), &root)
            .expect("self-host canary fixture must copy");
        Self { root }
    }

    fn generated_dir(&self) -> PathBuf {
        self.root.join("target/kobo-gen")
    }

    fn inspect_dir(&self) -> PathBuf {
        self.root.join("target/self-host-inspect")
    }
}

impl Drop for CanaryCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn self_host_canary_builds_runs_and_preserves_compiler_shaped_behavior() {
    let case = CanaryCase::new();

    let build = run_kobo_in(&case.root, &["build"]);
    assert_success(&build, "kobo build self-host canary");

    assert_generated_shape(&case.generated_dir());
    assert_generated_rust_has_no_kobo_compiler_deps(&case.generated_dir());

    let run = run_cargo_in(&case.generated_dir(), &["run", "--quiet"]);
    assert_success(&run, "generated canary cargo run");
    assert_canary_output_is_content_sensitive(&run.stdout);
}

#[test]
fn inspect_clean_cargo_canary_is_a_runnable_cargo_project() {
    let case = CanaryCase::new();
    let out_dir = case.inspect_dir();
    let out_dir_text = out_dir.to_string_lossy().to_string();

    let inspect = run_kobo_in(
        &case.root,
        &[
            "inspect",
            "--clean",
            "--cargo",
            &out_dir_text,
            "src/main.kobo",
        ],
    );
    assert_success(&inspect, "kobo inspect --clean --cargo self-host canary");

    assert_generated_shape(&out_dir);
    assert_generated_rust_has_no_kobo_compiler_deps(&out_dir);

    let run = run_cargo_in(&out_dir, &["run", "--quiet"]);
    assert_success(&run, "inspect-generated canary cargo run");
    assert_canary_output_is_content_sensitive(&run.stdout);
}

#[test]
fn doctor_self_host_reports_ready_canary_from_project_shape() {
    let case = CanaryCase::new();

    let build = run_kobo_in(&case.root, &["build"]);
    assert_success(&build, "kobo build before doctor readiness check");

    let ready = run_kobo_in(&case.root, &["doctor", "--self-host", "--json"]);
    assert_success(&ready, "doctor --self-host ready canary");
    let ready_json = parse_json(&ready.stdout);
    assert_eq!(ready_json["schema_version"], 1);
    assert_eq!(ready_json["command"], "doctor --self-host");
    assert_eq!(ready_json["self_host"]["status"], "compatible");
    assert_json_array_contains(
        &ready_json["self_host"]["signals"],
        "multi-file-kobo-project",
    );
    assert_json_array_contains(&ready_json["self_host"]["signals"], "library-root");
    assert_json_array_contains(&ready_json["self_host"]["signals"], "generated-cargo-build");
    assert!(
        ready_json["self_host"]["blockers"]
            .as_array()
            .expect("blockers should be an array")
            .is_empty(),
        "ready canary should not report blockers: {ready_json:#}"
    );
}

#[test]
fn doctor_self_host_generated_cargo_signal_requires_generated_project() {
    let case = CanaryCase::new();

    let before = run_kobo_in(&case.root, &["doctor", "--self-host", "--json"]);
    assert_success(&before, "doctor --self-host before generated cargo");
    let before_json = parse_json(&before.stdout);
    assert_json_array_does_not_contain(
        &before_json["self_host"]["signals"],
        "generated-cargo-build",
    );

    let build = run_kobo_in(&case.root, &["build"]);
    assert_success(&build, "kobo build creates generated cargo project");

    let after = run_kobo_in(&case.root, &["doctor", "--self-host", "--json"]);
    assert_success(&after, "doctor --self-host after generated cargo");
    let after_json = parse_json(&after.stdout);
    assert_json_array_contains(&after_json["self_host"]["signals"], "generated-cargo-build");
}

#[test]
fn doctor_self_host_reports_missing_library_blocker() {
    let case = CanaryCase::new();
    fs::remove_file(case.root.join("src/lib.kobo")).expect("lib.kobo should be removable");
    let blocked = run_kobo_in(&case.root, &["doctor", "--self-host", "--json"]);
    assert_success(&blocked, "doctor --self-host missing library canary");
    let blocked_json = parse_json(&blocked.stdout);
    assert_eq!(blocked_json["self_host"]["status"], "blocked");
    assert_json_array_contains(
        &blocked_json["self_host"]["blockers"],
        "missing-library-root",
    );
}

#[test]
fn doctor_self_host_reports_outside_scope_for_non_kobo_project() {
    let project = ScratchProject::new("self-host-outside-scope");

    let outside = run_kobo_in(&project.root, &["doctor", "--self-host", "--json"]);
    assert_success(&outside, "doctor --self-host outside-scope project");
    let outside_json = parse_json(&outside.stdout);
    assert_eq!(outside_json["self_host"]["status"], "outside-scope");
}

#[test]
fn doctor_self_host_reports_unsupported_dependency_form_blocker() {
    let case = CanaryCase::new();
    let manifest_path = case.root.join("Kobo.toml");
    let manifest = fs::read_to_string(&manifest_path).expect("manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace("[copy_types]", "invalid_dependency = true\n\n[copy_types]"),
    )
    .expect("manifest should write");

    let blocked = run_kobo_in(&case.root, &["doctor", "--self-host", "--json"]);
    assert_success(&blocked, "doctor --self-host unsupported dependency form");
    let blocked_json = parse_json(&blocked.stdout);
    assert_eq!(blocked_json["self_host"]["status"], "blocked");
    assert_json_array_contains(
        &blocked_json["self_host"]["blockers"],
        "unsupported-dependency-form",
    );
}

#[test]
fn self_host_canary_diagnostics_stay_on_kobo_source() {
    let case = CanaryCase::new();
    let bad_file = case.root.join("src/bad_query.kobo");
    fs::write(
        &bad_file,
        r#"
pub fn broken() {
    let answer =
}
"#,
    )
    .expect("bad canary file should write");

    let out = run_kobo_in(
        &case.root,
        &[
            "check",
            "--recover-parse",
            "--error-format=json",
            "src/bad_query.kobo",
        ],
    );
    assert!(
        !out.status.success(),
        "bad canary should fail through check\nstdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    let combined = format!("{}\n{}", out.stdout, out.stderr);
    assert!(
        combined.contains("K010") || combined.contains("schema_version"),
        "parse failure should use registry-backed JSON diagnostics\n{combined}"
    );
    assert!(
        combined.contains("bad_query.kobo"),
        "diagnostic should point at .kobo source\n{combined}"
    );
    assert!(
        !combined.contains("bad_query.rs"),
        "diagnostic must not expose generated .rs path\n{combined}"
    );
}

#[test]
fn self_host_compatibility_is_not_filename_or_constant_theater() {
    let workspace = workspace_root();
    let mut sources = String::new();
    for path in [
        "bin/kobo-cli/src/commands/doctor.rs",
        "crates/compiler/kobo-driver/src/multi_file.rs",
        "crates/compiler/kobo-codegen/src/cargo_gen.rs",
        "crates/compiler/kobo-driver/src/query.rs",
    ] {
        sources.push_str(&fs::read_to_string(workspace.join(path)).expect("source should read"));
        sources.push('\n');
    }

    assert!(
        !sources.contains("kobo-self-host-canary") || sources.contains("package_name"),
        "production code must not special-case the canary package name"
    );
    assert!(
        !sources.contains("self_host_canary=ok"),
        "production code must not print the canary success string"
    );
    assert!(
        !sources.contains("status: \"compatible\""),
        "doctor status must be computed from project shape, not fixed"
    );
}

fn assert_generated_shape(generated_dir: &Path) {
    for path in [
        "Cargo.toml",
        "src/main.rs",
        "src/lib.rs",
        "src/span.rs",
        "src/diagnostic.rs",
        "src/registry.rs",
        "src/query.rs",
    ] {
        assert!(
            generated_dir.join(path).is_file(),
            "generated project must contain {path} under {}",
            generated_dir.display()
        );
    }

    let cargo_toml = fs::read_to_string(generated_dir.join("Cargo.toml"))
        .expect("generated Cargo.toml should read");
    for expected in [
        "name = \"kobo-self-host-canary\"",
        "version = \"0.8.5\"",
        "edition = \"2021\"",
        "self_host_support",
        "path = \"../../support\"",
    ] {
        assert!(
            cargo_toml.contains(expected),
            "generated Cargo.toml missing {expected:?}\n{cargo_toml}"
        );
    }

    for forbidden in ["tokio", "async-std"] {
        assert!(
            !cargo_toml.contains(forbidden),
            "non-async self-host canary must not receive unrequested remote executor dependency {forbidden:?}\n{cargo_toml}"
        );
    }
}

fn assert_generated_rust_has_no_kobo_compiler_deps(generated_dir: &Path) {
    for file in collect_rs_files(generated_dir) {
        let source = fs::read_to_string(&file).expect("generated Rust should read");
        for forbidden in [
            "#[kobo::",
            "Owned<",
            "kobo_ir",
            "kobo_driver",
            "kobo_parser",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must not contain {forbidden:?}\n{source}",
                file.display()
            );
        }
    }
}

fn assert_canary_output_is_content_sensitive(stdout: &str) {
    for expected in [
        "self_host_canary=ok",
        "registry=K0107:unmodeled-external-boundary",
        "overlap=true",
        "support_salt=85",
        "query_changed=true",
        "source_hash_changed=true",
    ] {
        assert!(
            stdout.contains(expected),
            "canary output missing {expected:?}\nstdout:\n{stdout}"
        );
    }
}

fn run_kobo_in(dir: &Path, args: &[&str]) -> CmdOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("kobo command should start");
    cmd_output(output)
}

fn run_cargo_in(dir: &Path, args: &[&str]) -> CmdOutput {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("cargo command should start");
    cmd_output(output)
}

fn cmd_output(output: std::process::Output) -> CmdOutput {
    CmdOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn assert_success(output: &CmdOutput, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|error| panic!("stdout should be JSON: {error}\nstdout:\n{stdout}"))
}

fn assert_json_array_contains(value: &Value, expected: &str) {
    let array = value.as_array().expect("value should be array");
    assert!(
        array.iter().any(|item| item.as_str() == Some(expected)),
        "array should contain {expected:?}: {value:#}"
    );
}

fn assert_json_array_does_not_contain(value: &Value, unexpected: &str) {
    let array = value.as_array().expect("value should be array");
    assert!(
        array.iter().all(|item| item.as_str() != Some(unexpected)),
        "array should not contain {unexpected:?}: {value:#}"
    );
}

fn collect_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rs_files_into(root, &mut files);
    files
}

fn collect_rs_files_into(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("directory should read") {
        let path = entry.expect("entry should read").path();
        if path.is_dir() {
            collect_rs_files_into(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn copy_dir_all(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should exist")
}

struct ScratchProject {
    root: PathBuf,
}

impl ScratchProject {
    fn new(name: &str) -> Self {
        let workspace = workspace_root();
        let unique = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace
            .join("target-test-fixtures")
            .join(format!("{name}-{}-{unique}", std::process::id()));
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        fs::create_dir_all(&root).expect("scratch project root should create");
        Self { root }
    }
}

impl Drop for ScratchProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
