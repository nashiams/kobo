mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(90);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn demo_file(relative: &str) -> PathBuf {
    let path = repo_root().join(relative);
    assert!(path.is_file(), "{relative} should be a shipped demo source");
    path
}

fn copy_demo(project: &TestProject, relative: &str) -> PathBuf {
    let source = demo_file(relative);
    let contents = fs::read_to_string(&source).expect("demo source should be readable");
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .expect("demo source should have a UTF-8 name");
    project.write(&format!("src/{name}"), &contents)
}

fn run_gateway_test(
    project: &TestProject,
    file: &Path,
    target: &str,
    extra_args: &[String],
    expect_success: bool,
) -> (CliOutput, PathBuf, Value) {
    let mut args = vec![
        s("test"),
        s("--sim"),
        s("deep"),
        s("--engine"),
        s("both"),
        s("--seed"),
        s("1318"),
        s("--witness-dir"),
        s(".kobo/witnesses"),
        s("--target"),
        s(target),
        s("--error-format=json"),
    ];
    args.extend_from_slice(extra_args);
    args.push(path_arg(file));
    let output = run_kobo(&args, &project.root);
    if expect_success {
        assert_success(&output, "async gateway demo scenario");
    } else {
        assert_failure(&output, "async gateway demo scenario");
    }
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("async gateway demo should emit a witness");
    let witness =
        serde_json::from_str(&std::fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    (output, witness_path, witness)
}

#[test]
fn async_gateway_catches_cancellation_failure() {
    let project = TestProject::new("model-async-gateway-cancel");
    let demo = copy_demo(&project, "examples/async_gateway/cancellation_failure.kobo");

    let (output, _witness_path, witness) = run_gateway_test(
        &project,
        &demo,
        "cancellation_failure",
        &[s("--inject"), s("cancel")],
        false,
    );
    assert_contains(&output.combined(), "K0100", "cancel should fail liveness");
    assert_contains(
        &witness["events"].to_string(),
        "scheduler-cancel-path",
        "witness should retain cancellation history",
    );
    assert_eq!(witness["flagship_demo"]["name"], "async_gateway");
}

#[test]
fn async_gateway_catches_orphan_task_failure() {
    let project = TestProject::new("model-async-gateway-orphan");
    let demo = copy_demo(&project, "examples/async_gateway/orphan_task_failure.kobo");

    let (output, _witness_path, witness) =
        run_gateway_test(&project, &demo, "orphan_task_failure", &[], false);
    assert_contains(
        &output.combined(),
        "no_orphan_tasks",
        "orphan task invariant should be named in the failure",
    );
    assert_eq!(witness["invariant_checks"][0]["status"], "failed");
}

#[test]
fn async_gateway_catches_request_token_failure() {
    let project = TestProject::new("model-async-gateway-token-failure");
    let demo = copy_demo(
        &project,
        "examples/async_gateway/request_token_failure.kobo",
    );

    let (_output, _witness_path, witness) =
        run_gateway_test(&project, &demo, "request_token_failure", &[], false);
    assert_eq!(witness["failure"]["mode"], "reply-open");
}

#[test]
fn async_gateway_reply_reject_cancel_inferred_without_manual_declarations() {
    let project = TestProject::new("model-async-gateway-inferred");
    let demo = copy_demo(&project, "examples/async_gateway/passing_history.kobo");
    let (_output, _witness_path, witness) =
        run_gateway_test(&project, &demo, "gateway_success", &[], true);

    let handler_reply = witness["inferred_obligations"]
        .as_array()
        .expect("inferred obligations should be present")
        .iter()
        .find(|entry| entry["template_id"].as_str() == Some("handler_reply"))
        .unwrap_or_else(|| {
            panic!(
                "missing handler_reply in {}",
                witness["inferred_obligations"]
            )
        });
    for action in ["reply", "reject", "cancel"] {
        assert_contains(
            &handler_reply["terminal_actions"].to_string(),
            action,
            "handler reply lifecycle should be inferred without manual declaration",
        );
    }
}

#[test]
fn async_gateway_emits_replayable_kwit_witness() {
    let project = TestProject::new("model-async-gateway-replay");
    let demo = copy_demo(&project, "examples/async_gateway/passing_history.kobo");
    let (_output, witness_path, witness) =
        run_gateway_test(&project, &demo, "gateway_success", &[], true);
    assert_eq!(witness["replay_guarantee"], "exact");
    assert_eq!(
        witness["flagship_demo"]["replayable_kwit"],
        Value::Bool(true)
    );
    assert_eq!(
        witness["flagship_demo"]["scheduler_preset"].as_str(),
        Some("async")
    );
    assert_eq!(
        witness["flagship_demo"]["evidence_inputs"]["source"], "compiler-scenario-program",
        "async gateway demo evidence should be derived from compiler scenario facts"
    );
    assert_contains(
        &witness["flagship_demo"]["evidence_inputs"]["modeled_boundaries"].to_string(),
        "ward.task",
        "async gateway demo evidence should expose concrete modeled boundaries",
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_success(&replay, "async gateway demo witness should replay");
    assert_contains(
        &replay.combined(),
        r#""replay":"exact""#,
        "async gateway replay should validate exact witness evidence",
    );
}

#[test]
fn async_gateway_clean_rust_output_builds() {
    let project = TestProject::new("model-async-gateway-clean-rust");
    let file = copy_demo(&project, "examples/async_gateway/clean_exit.kobo");
    let out_dir = project.root.join("target/gateway-clean");
    let output = run_kobo(
        &[
            s("inspect"),
            s("--clean"),
            s("--cargo"),
            path_arg(&out_dir),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "async gateway clean Rust cargo export");

    let cargo = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(out_dir.join("Cargo.toml"))
        .output()
        .expect("cargo check should launch for clean Rust output");
    let cargo_output = CliOutput {
        status: cargo.status,
        stdout: String::from_utf8_lossy(&cargo.stdout).to_string(),
        stderr: String::from_utf8_lossy(&cargo.stderr).to_string(),
    };
    assert_success(
        &cargo_output,
        "async gateway clean Rust output should build",
    );
}
