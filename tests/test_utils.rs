/// Compile a .kobo fixture to .rs output, then compile the .rs with rustc,
/// and run the resulting binary. Returns (stdout, stderr, exit_code).
///
/// Uses `kobo run <fixture>` which handles the full pipeline:
/// parse → transform → analysis → codegen → rustc → execute.
#[allow(dead_code)]
pub fn assert_codegen_and_run(fixture_path: &str) -> (String, String, i32) {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let kobo_bin = workspace_root.join("target").join("debug").join("kobo");

    let output = std::process::Command::new(&kobo_bin)
        .arg("run")
        .arg(fixture_path)
        .current_dir(workspace_root)
        .output()
        .unwrap_or_else(|e| panic!("failed to run kobo binary at {}: {e}", kobo_bin.display()));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);
    (stdout, stderr, exit_code)
}

/// Run `kobo codegen <fixture>` and snapshot the generated Rust source.
#[allow(dead_code)]
pub fn assert_codegen_snapshot(fixture_path: &str) -> String {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let kobo_bin = workspace_root.join("target").join("debug").join("kobo");

    let output = std::process::Command::new(&kobo_bin)
        .arg("codegen")
        .arg(fixture_path)
        .current_dir(workspace_root)
        .output()
        .unwrap_or_else(|e| panic!("failed to run kobo binary at {}: {e}", kobo_bin.display()));

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Run `kobo check <fixture>` and return stderr for snapshot comparison.
#[allow(dead_code)]
pub fn assert_stderr_snapshot(fixture_path: &str) -> String {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let kobo_bin = workspace_root.join("target").join("debug").join("kobo");

    let output = std::process::Command::new(&kobo_bin)
        .arg("check")
        .arg(fixture_path)
        .current_dir(workspace_root)
        .output()
        .unwrap_or_else(|e| panic!("failed to run kobo binary at {}: {e}", kobo_bin.display()));

    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Assert that a file's contents are byte-identical to a baseline file.
/// When `KOBO_UPDATE_SNAPSHOTS=1` is set, writes the actual content as the new baseline.
#[allow(dead_code)]
pub fn assert_byte_identical_to_baseline(actual_path: &str, baseline_path: &str) {
    let actual = std::fs::read(actual_path)
        .unwrap_or_else(|e| panic!("failed to read actual file {actual_path}: {e}"));

    if std::env::var("KOBO_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        if let Some(parent) = std::path::Path::new(baseline_path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(baseline_path, &actual)
            .unwrap_or_else(|e| panic!("failed to write baseline {baseline_path}: {e}"));
        return;
    }

    let baseline = std::fs::read(baseline_path)
        .unwrap_or_else(|e| panic!("failed to read baseline {baseline_path}: {e}"));

    assert!(
        actual == baseline,
        "byte mismatch: {actual_path} differs from baseline {baseline_path} ({} vs {} bytes)",
        actual.len(),
        baseline.len(),
    );
}
