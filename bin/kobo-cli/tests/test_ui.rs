use std::path::{Path, PathBuf};
use std::process::Command;

struct KoboOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

#[test]
fn check_k0001_fixture_matches_snapshot() {
    let fixture = workspace_root()
        .join("tests")
        .join("ui")
        .join("K0001_use_after_move.kobo");
    let output = run_kobo_check(&fixture);

    assert!(!output.status.success(), "check should fail");
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_check__K0001_use_after_move", output.stderr);
    });
}

#[test]
fn check_k0002_fixture_matches_snapshot() {
    let fixture = workspace_root()
        .join("tests")
        .join("ui")
        .join("K0002_mutable_borrow_conflict.kobo");
    let output = run_kobo_check(&fixture);

    assert!(!output.status.success(), "check should fail");
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_check__K0002_mutable_borrow_conflict", output.stderr);
    });
}

#[test]
fn check_k0025_fixture_matches_snapshot() {
    let fixture = workspace_root()
        .join("tests")
        .join("ui")
        .join("K0025_hint_ignored.kobo");
    let output = run_kobo_check(&fixture);

    assert!(
        output.status.success(),
        "check should succeed for warning-only diagnostics"
    );
    assert!(output.stderr.contains("warning[K0025]"));
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_check__K0025_hint_ignored", output.stderr);
    });
}

#[test]
fn run_k0001_fixture_stops_at_analysis() {
    let fixture = workspace_root()
        .join("tests")
        .join("ui")
        .join("K0001_use_after_move.kobo");
    let output = run_kobo_command("run", &fixture);

    assert!(!output.status.success(), "run should fail");
    assert!(output.stderr.contains("error[K0001]"));
    assert!(!output.stderr.contains("error[K0099]"));
}

#[test]
fn run_k0002_fixture_stops_at_analysis() {
    let fixture = workspace_root()
        .join("tests")
        .join("ui")
        .join("K0002_mutable_borrow_conflict.kobo");
    let output = run_kobo_command("run", &fixture);

    assert!(!output.status.success(), "run should fail");
    assert!(output.stderr.contains("error[K0002]"));
    assert!(!output.stderr.contains("RefCell already borrowed"));
}

fn run_kobo_check(fixture_path: &Path) -> KoboOutput {
    run_kobo_command("check", fixture_path)
}

fn run_kobo_command(command: &str, fixture_path: &Path) -> KoboOutput {
    let workspace_root = workspace_root();
    let relative_fixture = fixture_path
        .strip_prefix(&workspace_root)
        .expect("fixture should live under workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args([
            command,
            relative_fixture.to_str().expect("utf8 fixture path"),
        ])
        .current_dir(&workspace_root)
        .output()
        .expect("kobo command should run");

    KoboOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}

// ---------------------------------------------------------------------------
// v0.4 — K0080-P structural warning tests
// ---------------------------------------------------------------------------

#[test]
fn check_k0080_p1_emits_note() {
    let fixture = workspace_root()
        .join("tests")
        .join("ui")
        .join("K0080_P1_node.kobo");
    let output = run_kobo_check(&fixture);

    // K0080-P1 is advisory-only: check succeeds (no error), but note is present.
    assert!(output.status.success(), "K0080-P1 is note-only, check should succeed");
    assert!(
        output.stderr.contains("note[K0080-P1]"),
        "stderr must contain note[K0080-P1], got:\n{}",
        output.stderr
    );
    // Severity contract: must NOT be warning or error.
    assert!(
        !output.stderr.contains("warning[K0080-P1]"),
        "K0080-P1 must not be a warning"
    );
    assert!(
        !output.stderr.contains("error[K0080-P1]"),
        "K0080-P1 must not be an error"
    );
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_check__K0080_P1_node", output.stderr);
    });
}

#[test]
fn check_k0080_p1_suppressed_by_known_debt() {
    let fixture = workspace_root()
        .join("tests")
        .join("fixtures")
        .join("k0080_p1_suppressed.kobo");
    let output = run_kobo_check(&fixture);

    assert!(output.status.success(), "suppressed K0080-P1 check should succeed");
    assert!(
        !output.stderr.contains("K0080-P1"),
        "suppressed binding must produce zero K0080-P1 notes, got:\n{}",
        output.stderr
    );
}

#[test]
fn check_k0080_p2_tree_emits_note() {
    let fixture = workspace_root()
        .join("tests")
        .join("fixtures")
        .join("k0080_p2_tree.kobo");
    let output = run_kobo_check(&fixture);

    assert!(output.status.success(), "K0080-P2 is note-only, check should succeed");
    assert!(
        output.stderr.contains("note[K0080-P2]"),
        "stderr must contain note[K0080-P2], got:\n{}",
        output.stderr
    );
}

#[test]
fn check_k0080_p4_self_ref_emits_note() {
    let fixture = workspace_root()
        .join("tests")
        .join("fixtures")
        .join("k0080_p4_self_ref.kobo");
    let output = run_kobo_check(&fixture);

    assert!(output.status.success(), "K0080-P4 is note-only, check should succeed");
    assert!(
        output.stderr.contains("note[K0080-P4]"),
        "stderr must contain note[K0080-P4], got:\n{}",
        output.stderr
    );
}

// ---------------------------------------------------------------------------
// v0.4 — kobo debt command tests
// ---------------------------------------------------------------------------

#[test]
fn debt_summary_is_single_line() {
    let fixture = workspace_root()
        .join("tests")
        .join("fixtures")
        .join("debt_report_full.kobo");
    let output = run_kobo_command_with_args(
        "debt",
        &fixture,
        &["--summary"],
    );

    assert!(output.status.success(), "debt --summary should succeed");
    let lines: Vec<_> = output.stdout.lines().collect();
    assert_eq!(lines.len(), 1, "debt --summary must output exactly one line, got: {:?}", lines);
}

#[test]
fn debt_json_has_schema_version() {
    let fixture = workspace_root()
        .join("tests")
        .join("fixtures")
        .join("debt_report_full.kobo");
    let output = run_kobo_command_with_args(
        "debt",
        &fixture,
        &["--json"],
    );

    assert!(output.status.success(), "debt --json should succeed");
    assert!(
        output.stdout.contains("\"schema_version\""),
        "JSON output must contain schema_version field"
    );
    assert!(
        output.stdout.contains("\"schema_version\": 1"),
        "schema_version must be 1, got:\n{}",
        output.stdout
    );

    // Must be valid JSON that round-trips.
    let parsed: serde_json::Value = serde_json::from_str(&output.stdout)
        .expect("debt --json output must be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
}

fn run_kobo_command_with_args(command: &str, fixture_path: &Path, extra_args: &[&str]) -> KoboOutput {
    let workspace_root = workspace_root();
    let relative_fixture = fixture_path
        .strip_prefix(&workspace_root)
        .expect("fixture should live under workspace root");
    let mut args = vec![command, relative_fixture.to_str().expect("utf8 fixture path")];
    args.extend_from_slice(extra_args);
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(&args)
        .current_dir(&workspace_root)
        .output()
        .expect("kobo command should run");

    KoboOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}
