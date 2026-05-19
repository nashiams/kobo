mod v09_common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(90);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn async_gateway_inferred_source() -> &'static str {
    r#"
struct AsyncGateway {}
struct RequestToken {}

impl RequestToken {
    fn reply(self) {}
    fn reject(self) {}
    fn cancel(self) {}
}

#[kobo::handler]
async fn gateway_handler(token: RequestToken) {
    token.reject();
}

#[kobo::scenario(profile = "async")]
async fn gateway_success() {
    let _gateway = AsyncGateway {};
    gateway_handler(RequestToken {}).await;
    ward.task();
}
"#
}

fn async_gateway_manual_token_source(scenario: &str) -> String {
    format!(
        r#"
struct AsyncGateway {{}}

#[kobo::must_call(reply | reject | cancel)]
struct RequestToken {{}}

{scenario}
"#
    )
}

fn async_gateway_orphan_source() -> &'static str {
    r#"
struct AsyncGateway {}

// kobo:invariant no_orphan_tasks { never scheduler-task-enqueued }
#[kobo::scenario(profile = "async")]
fn orphan_task_failure() {
    let _gateway = AsyncGateway {};
    ward.task();
}
"#
}

fn gateway_clean_source() -> String {
    r#"
struct AsyncGateway {}
struct RequestToken {}

impl RequestToken {
    fn reply(self) {}
    fn reject(self) {}
    fn cancel(self) {}
}

fn gateway_clean_path() {
    let _gateway = AsyncGateway {};
    let token = RequestToken {};
    token.reply();
}

fn main() {}
"#
    .to_owned()
}

fn run_gateway_test(
    project: &TestProject,
    source: &str,
    target: &str,
    extra_args: &[String],
    expect_success: bool,
) -> (CliOutput, PathBuf, Value) {
    let file = project.main_file(source);
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
    args.push(path_arg(&file));
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
    let project = TestProject::new("v13-async-gateway-cancel");
    let source = async_gateway_manual_token_source(
        r#"
#[kobo::scenario(profile = "async")]
fn cancellation_failure() {
    let _gateway = AsyncGateway {};
    let token = RequestToken {};
    ward.task();
    token.reply();
}
"#,
    );

    let (output, _witness_path, witness) = run_gateway_test(
        &project,
        &source,
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
    let project = TestProject::new("v13-async-gateway-orphan");

    let (output, _witness_path, witness) = run_gateway_test(
        &project,
        async_gateway_orphan_source(),
        "orphan_task_failure",
        &[],
        false,
    );
    assert_contains(
        &output.combined(),
        "no_orphan_tasks",
        "orphan task invariant should be named in the failure",
    );
    assert_eq!(witness["invariant_checks"][0]["status"], "failed");
}

#[test]
fn async_gateway_catches_request_token_failure() {
    let project = TestProject::new("v13-async-gateway-token-failure");
    let source = async_gateway_manual_token_source(
        r#"
#[kobo::scenario(profile = "async")]
fn request_token_failure() {
    let _gateway = AsyncGateway {};
    let token = RequestToken {};
    let _lost = token;
}
"#,
    );

    let (output, _witness_path, witness) =
        run_gateway_test(&project, &source, "request_token_failure", &[], false);
    assert_contains(&output.combined(), "reply", "diagnostic should name reply");
    assert_eq!(witness["failure"]["mode"], "unresolved-reply");
}

#[test]
fn async_gateway_reply_reject_cancel_inferred_without_manual_declarations() {
    let project = TestProject::new("v13-async-gateway-inferred");
    let (_output, _witness_path, witness) = run_gateway_test(
        &project,
        async_gateway_inferred_source(),
        "gateway_success",
        &[],
        true,
    );

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
    let project = TestProject::new("v13-async-gateway-replay");
    let (_output, witness_path, witness) = run_gateway_test(
        &project,
        async_gateway_inferred_source(),
        "gateway_success",
        &[],
        true,
    );
    assert_eq!(witness["replay_guarantee"], "exact");
    assert_eq!(
        witness["flagship_demo"]["replayable_kwit"],
        Value::Bool(true)
    );
    assert_eq!(
        witness["flagship_demo"]["scheduler_preset"].as_str(),
        Some("async")
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
    let project = TestProject::new("v13-async-gateway-clean-rust");
    let clean_source = gateway_clean_source();
    let file = project.main_file(&clean_source);
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
