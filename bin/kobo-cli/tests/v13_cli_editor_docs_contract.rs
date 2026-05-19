mod v09_common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo_with_timeout, s,
    CliOutput, TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(90);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn clean_exit_source() -> &'static str {
    r#"
fn double(value: i32) -> i32 {
    value * 2
}

fn main() {
    println!("{}", double(21));
}
"#
}

#[test]
fn complete_cli_surface_documents_required_commands() {
    let project = TestProject::new("v13-cli-help");
    let help = run_kobo(&[s("--help")], &project.root);
    assert_success(&help, "top-level help should render");
    for command in [
        "run", "build", "check", "inspect", "migrate", "debt", "perf", "sim", "replay",
    ] {
        assert_contains(
            &help.stdout,
            command,
            "top-level help should document required command",
        );
    }

    let inspect = run_kobo(&[s("inspect"), s("--help")], &project.root);
    assert_success(&inspect, "inspect help should render");
    for flag in ["--clean", "--cargo", "--scenario-metadata"] {
        assert_contains(
            &inspect.stdout,
            flag,
            "inspect help should document subflag",
        );
    }

    let debt = run_kobo(&[s("debt"), s("--help")], &project.root);
    assert_success(&debt, "debt help should render");
    for flag in ["--json", "--summary", "--watch"] {
        assert_contains(&debt.stdout, flag, "debt help should document subflag");
    }

    let sim = run_kobo(&[s("sim"), s("--help")], &project.root);
    assert_success(&sim, "sim help should render");
    for subcommand in ["init", "backends", "scout"] {
        assert_contains(
            &sim.stdout,
            subcommand,
            "sim help should document subcommand",
        );
    }
}

#[test]
fn inspect_clean_and_inspect_cargo_preserve_clean_rust_exit_ramp() {
    let project = TestProject::new("v13-clean-exit-ramp");
    let file = project.main_file(clean_exit_source());

    let clean = run_kobo(
        &[s("inspect"), s("--clean"), path_arg(&file)],
        &project.root,
    );
    assert_success(&clean, "inspect --clean should render clean Rust");
    assert_contains(
        &clean.stdout,
        "fn main()",
        "clean output should retain main",
    );
    assert_not_contains(
        &clean.stdout,
        "kobo::",
        "clean output should not depend on Kobo attributes",
    );

    let out_dir = project.root.join("target/clean-rust-cargo");
    let cargo_export = run_kobo(
        &[
            s("inspect"),
            s("--clean"),
            s("--cargo"),
            path_arg(&out_dir),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &cargo_export,
        "inspect --clean --cargo should export Cargo project",
    );

    let check = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(out_dir.join("Cargo.toml"))
        .output()
        .expect("cargo check should launch");
    assert_success(
        &command_output(check),
        "clean Rust Cargo output should build",
    );

    let clippy = Command::new("cargo")
        .arg("clippy")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(out_dir.join("Cargo.toml"))
        .arg("--")
        .arg("-D")
        .arg("warnings")
        .output()
        .expect("cargo clippy should launch");
    assert_success(
        &command_output(clippy),
        "clean Rust Cargo output should satisfy clippy warning baseline",
    );
}

fn command_output(output: std::process::Output) -> CliOutput {
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}
