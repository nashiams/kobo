use std::path::{Path, PathBuf};
use std::process::Command;

struct KoboOutput {
    status: std::process::ExitStatus,
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
        .args([command, relative_fixture.to_str().expect("utf8 fixture path")])
        .current_dir(&workspace_root)
        .output()
        .expect("kobo command should run");

    KoboOutput {
        status: output.status,
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
