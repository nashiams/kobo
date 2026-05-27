use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

const UC_FIXTURES: &[&str] = &[
    "uc1_video_pipeline.kobo",
    "uc2_kv_store.kobo",
    "uc3_api_gateway.kobo",
    "uc4_game_server.kobo",
];

struct FixtureCase {
    root: PathBuf,
    fixture_path: PathBuf,
}

impl FixtureCase {
    fn new(fixture_name: &str) -> Self {
        let workspace_root = workspace_root();
        let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace_root
            .join("target-test-fixtures")
            .join(format!("uc-acceptance-{}-{unique_id}", std::process::id()));

        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        fs::create_dir_all(&root).expect("fixture temp root should be creatable");

        let source_fixture = workspace_root
            .join("tests")
            .join("fixtures")
            .join(fixture_name);
        let fixture_path = root.join(fixture_name);
        fs::copy(&source_fixture, &fixture_path).expect("UC fixture should copy");

        Self { root, fixture_path }
    }

    fn cargo_output_dir(&self) -> PathBuf {
        self.root.join("generated-cargo")
    }
}

impl Drop for FixtureCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct CmdOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

#[test]
fn uc_fixtures_run_through_migrate_inspect_and_cargo_check() {
    for fixture_name in UC_FIXTURES {
        let case = FixtureCase::new(fixture_name);

        let first = run_kobo(
            ["migrate", "--dry-run", "--class", "--explain"],
            &case.fixture_path,
        );
        assert_success(&first, fixture_name, "first migrate dry-run");
        assert_migrate_check(&first, fixture_name);

        let second = run_kobo(
            ["migrate", "--dry-run", "--class", "--explain"],
            &case.fixture_path,
        );
        assert_success(&second, fixture_name, "second migrate dry-run");
        assert_eq!(
            first.stdout, second.stdout,
            "{fixture_name} migrate output must be stable across repeated production runs"
        );

        let review = run_kobo(
            ["migrate", "--dry-run", "--review", "--class", "--explain"],
            &case.fixture_path,
        );
        assert_success(&review, fixture_name, "review migrate dry-run");
        assert_migrate_check(&review, fixture_name);
        assert!(
            case.root.join(".kobo").join("decisions.toml").exists(),
            "{fixture_name} review run must leave a durable decisions artifact"
        );

        let cargo_dir = case.cargo_output_dir();
        let cargo_dir_string = cargo_dir.to_string_lossy().into_owned();
        let relative_fixture = relative_to_workspace(&case.fixture_path);
        let relative_fixture_string = relative_fixture.to_string_lossy().into_owned();
        let inspect = run_kobo_with_args(&[
            "inspect",
            "--clean",
            "--cargo",
            &cargo_dir_string,
            &relative_fixture_string,
        ]);
        assert_success(&inspect, fixture_name, "inspect --cargo");

        let source_map = case.fixture_path.with_extension("kobo.map");
        assert!(
            source_map.exists(),
            "{fixture_name} inspect must write a source map for solver provenance"
        );
        assert_solver_evidence(&source_map, fixture_name);

        let cargo_check = run_cargo_check(&case.cargo_output_dir());
        assert_success(&cargo_check, fixture_name, "generated cargo check");
    }
}

fn assert_migrate_check(output: &CmdOutput, fixture_name: &str) {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    assert!(
        combined.contains("solver outcome:"),
        "{fixture_name} must expose solver outcome\n{combined}"
    );
    assert!(
        combined.contains("graph fingerprint:"),
        "{fixture_name} must expose graph fingerprint\n{combined}"
    );
    assert!(
        combined.contains("decision class:"),
        "{fixture_name} must expose decision class\n{combined}"
    );
    assert!(
        combined.contains("silent_decisions: 0"),
        "{fixture_name} must not make silent ownership decisions\n{combined}"
    );
    assert!(
        !combined.contains("[K0080]"),
        "{fixture_name} must not carry unresolved ownership conflicts\n{combined}"
    );
    assert!(
        !combined.contains("[K0081]"),
        "{fixture_name} must fit within the production solver cap\n{combined}"
    );
}

fn assert_solver_evidence(source_map_path: &Path, fixture_name: &str) {
    let source_map = fs::read_to_string(source_map_path).expect("source map should read");
    let parsed: Value = serde_json::from_str(&source_map).unwrap_or_else(|error| {
        panic!("{fixture_name} source map must be valid JSON: {error}\n{source_map}")
    });

    let evidence = parsed
        .get("solver_evidence")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{fixture_name} must include top-level solver_evidence"));
    let fingerprint = evidence
        .get("graph_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{fixture_name} must include graph_fingerprint"));
    assert!(
        fingerprint.len() >= 16,
        "{fixture_name} graph fingerprint must be non-decorative: {fingerprint}"
    );
    let budget = evidence
        .get("budget")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{fixture_name} must include solver budget evidence"));
    assert_eq!(
        budget.get("max_cluster_size").and_then(Value::as_u64),
        Some(2048),
        "{fixture_name} source map must record the production cluster cap"
    );

    let mappings = parsed
        .get("x_kobo_mappings")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{fixture_name} must include mapping evidence"));
    assert!(
        !mappings.is_empty(),
        "{fixture_name} must carry per-binding mapping provenance"
    );
}

fn assert_success(output: &CmdOutput, fixture_name: &str, step: &str) {
    assert!(
        output.status.success(),
        "{fixture_name} {step} failed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn run_kobo<const N: usize>(args: [&str; N], fixture_path: &Path) -> CmdOutput {
    let relative_fixture = relative_to_workspace(fixture_path);
    let relative_fixture_string = relative_fixture.to_string_lossy().into_owned();
    let mut command_args: Vec<&str> = args.to_vec();
    command_args.push(&relative_fixture_string);
    run_kobo_with_args(&command_args)
}

fn run_kobo_with_args(args: &[&str]) -> CmdOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("kobo command should run");

    CmdOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn run_cargo_check(project_dir: &Path) -> CmdOutput {
    let output = Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(project_dir.join("Cargo.toml"))
        .current_dir(workspace_root())
        .output()
        .expect("cargo check should run");

    CmdOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn relative_to_workspace(path: &Path) -> PathBuf {
    path.strip_prefix(workspace_root())
        .expect("path should live under workspace root")
        .to_path_buf()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}
