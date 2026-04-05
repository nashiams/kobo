/// Compile a .kobo fixture to .rs output, then compile the .rs with rustc,
/// and run the resulting binary. Returns (stdout, stderr, exit_code).
pub fn assert_codegen_and_run(_fixture_path: &str) -> (String, String, i32) {
    // 1. kobo compile <fixture> -o /tmp/test_output.rs
    // 2. rustc /tmp/test_output.rs -o /tmp/test_binary
    // 3. /tmp/test_binary
    // 4. return (stdout, stderr, exit_code)
    todo!("implement after Group A-H fixes are verified")
}
