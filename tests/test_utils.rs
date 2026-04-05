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
