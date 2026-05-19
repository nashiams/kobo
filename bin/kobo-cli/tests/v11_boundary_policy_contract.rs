mod v09_common;

use std::path::Path;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

fn write_path_crate(project: &TestProject, name: &str) {
    project.write(
        &format!("{name}/Cargo.toml"),
        &format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
"#
        ),
    );
    project.write(
        &format!("{name}/src/lib.rs"),
        r#"
pub struct Gateway {}
pub fn charge() -> Gateway { Gateway {} }
"#,
    );
}

#[test]
fn replay_critical_boundary_uses_imported_function_and_cargo_identity() {
    let project = TestProject::new("v11-boundary-imported-function");
    write_path_crate(&project, "payments");
    project.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "boundary_consumer"
version = "0.1.0"
edition = "2021"

[dependencies]
payments = {{ path = "{}" }}
"#,
            project
                .root
                .join("payments")
                .display()
                .to_string()
                .replace('\\', "/")
        ),
    );
    project.write("src/main.rs", "fn main() {}\n");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "debt"

[[ecosystem.crate]]
name = "payments"
policy = "activity"
reason = "payment gateway runs outside deterministic replay"
"#,
    );
    let file = project.main_file(
        r#"
use payments::charge;

fn main() {
    let _gateway = charge();
}
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "replay-critical policy should be derived from the imported external call",
    );
    assert_contains(
        &output.stdout,
        r#""crate":"payments""#,
        "boundary evidence should identify the Cargo package",
    );
    assert_contains(
        &output.stdout,
        r#""policy":"activity""#,
        "project policy should apply to imported external calls, not just Client::new fixtures",
    );
    assert_contains(
        &output.stdout,
        r#""package_version":"0.1.0""#,
        "boundary evidence should carry package version identity",
    );
    assert_contains(
        &output.stdout,
        r#""package_identity_source":"cargo-metadata""#,
        "boundary evidence should use resolved Cargo metadata for package identity",
    );
    assert_contains(
        &output.stdout,
        r#""package_source":"workspace-or-path""#,
        "path packages resolved through cargo metadata should keep source identity honest",
    );
    assert_contains(
        &output.stdout,
        r#""package_id":"path+"#,
        "boundary evidence should carry the resolved Cargo package id",
    );
}

#[test]
fn lsp_boundary_actions_include_all_v11_policy_choices() {
    let project = TestProject::new("v11-boundary-lsp-actions");
    write_path_crate(&project, "payments");
    project.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "boundary_lsp"
version = "0.1.0"
edition = "2021"

[dependencies]
payments = {{ path = "{}" }}
"#,
            project
                .root
                .join("payments")
                .display()
                .to_string()
                .replace('\\', "/")
        ),
    );
    project.write("src/main.rs", "fn main() {}\n");
    let file = project.main_file(
        r#"
use payments::charge;

fn main() {
    let _gateway = charge();
}
"#,
    );

    let output = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&file),
            s("--include-actions"),
        ],
        &project.root,
    );

    assert_success(&output, "LSP diagnostics should render boundary actions");
    assert_contains(
        &output.stdout,
        "Boundary policy: typed",
        "typed policy must be offered by LSP actions",
    );
    assert_contains(
        &output.stdout,
        "Boundary policy: activity",
        "activity policy must be offered by LSP actions",
    );
}
