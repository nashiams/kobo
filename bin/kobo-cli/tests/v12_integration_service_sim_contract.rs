mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg,
    run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V12_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V12_TIMEOUT)
}

fn write_activity_declaration(project: &TestProject) {
    project.write(
        "mailer.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "mailer"
version = "0.1"
source = "bindgen"

[[activity]]
path = "mailer::send_email"
retry = "retry-with-backoff"
idempotency = "message-id"
result = "record"
compensation = "cancel-email"
"#,
    );
}

fn integration_project(label: &str) -> (TestProject, PathBuf) {
    let project = TestProject::new(label);
    write_activity_declaration(&project);
    project.write(
        "Kobo.toml",
        r#"[runtime.profile]
service_buffer = 17
service_backpressure = "block-on-full"
scheduler = "pct-random-bounded"
record = "recorded-boundary-io"
activity = "retry-idempotency"
cancellation = "cooperative"
scenario_event_budget = 128
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "config_source", policy = "record", reason = "record service config reads")]
use config_source::config_value;

#[kobo::boundary(crate = "mailer", policy = "activity", reason = "email side effect runs outside replay")]
use mailer::send_email;
use std::rc::Rc;

struct Gateway {}

struct Request {
    id: u64,
}

struct Response {
    status: u16,
}

struct HandlerError {}

impl Request {
    fn reply(&self) {}
    fn reject(&self) {}
    fn cancel(&self) {}
}

#[kobo::service]
impl Gateway {
    async fn submit(&self, request: Request) -> Response {
        Response { status: 202 }
    }

    async fn refresh(&self, key: String, force: bool) -> Response {
        let _seen = (key, force);
        Response { status: 204 }
    }
}

#[kobo::handler]
async fn handle(request: Request) -> Result<Response, HandlerError> {
    request.reply();
    Ok(Response { status: 200 })
}

fn cpu_hash(value: u64) -> u64 {
    value.wrapping_mul(31).rotate_left(3)
}

fn crunch(values: Vec<u64>) {
    #[kobo::parallel]
    for value in values.iter() {
        let _hashed = cpu_hash(*value);
    }
}

#[kobo::scenario(profile = "async")]
async fn product_loop() {
    let _config = config_value("region-a");
    let _sent = send_email("receipt-123");
    let (client, worker) = GatewayService::start(Gateway {});
    let _submitted = client.submit(Request { id: 10 }).await.expect("submit");
    let _refreshed = client.refresh("cache".to_owned(), true).await.expect("refresh");
    client.shutdown_and_wait(worker).await.expect("shutdown");
    let local_state = Rc::new(5_u64);
    spawn local {
        let _local_value = *local_state;
        ward.task();
    };
    ward.task();
    let request = Request { id: 1 };
    handle(request).await;
    ward.network.drop_message("client");
}
"#,
    );
    project.write(
        "Cargo.toml",
        r#"[package]
name = "integrated_service_sim"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = "0.12"
"#,
    );
    project.write(
        "src/main.rs",
        r#"
fn main() {
    let _request = reqwest::get("https://example.test/integration");
}
"#,
    );
    (project, file)
}

fn first_witness(project: &TestProject) -> Value {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("integrated scenario should write a witness");
    serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

#[test]
fn integrated_service_handler_record_activity_spawn_local_and_parallel_are_visible() {
    let (project, file) = integration_project("v12-integrated-inspect");
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);

    assert_success(&output, "integrated fixture should inspect");
    let generated = output.combined();
    for expected in [
        "enum GatewayMessage",
        "async fn serve(",
        "service.refresh(key, force).await",
        "KoboServiceScenarioHookEvent",
        "KoboHandlerOutcome",
        "run_registered_cleanup",
        "record_reply",
        "tokio::task::spawn_local",
        "values.par_iter()",
        "kobo: runtime profile service_buffer=17",
    ] {
        assert_contains(
            &generated,
            expected,
            "integrated inspect output should expose each productized surface",
        );
    }
    assert_not_contains(
        &generated,
        "shuttle::",
        "generated user Rust must not import backend scheduler crates",
    );
}

#[test]
fn integrated_service_sim_witness_carries_full_product_loop_evidence() {
    let (project, file) = integration_project("v12-integrated-witness");
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--target"),
            s("product_loop"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "integrated product loop should produce witness evidence",
    );
    let witness = first_witness(&project);
    let text = witness.to_string();
    for expected in [
        r#""service_runtime""#,
        r#""handler_lifecycle""#,
        "Gateway",
        "refresh",
        "hook_events",
        "runtime_hook_events",
        "handle",
        "parallel_lowering",
        "task_local_zones",
        "local_state",
        "boundary-record",
        "recorded-boundary-io",
        "boundary-activity",
        "activity-result",
        "ward.task.local",
        "runtime_profile",
        "runtime_profile_hash",
        "network-drop-message",
    ] {
        assert_contains(
            &text,
            expected,
            "integrated witness should carry service, handler, replay, and scheduler evidence",
        );
    }
    assert_eq!(
        witness["service_runtime"]["services"][0]["buffer"], 17,
        "runtime profile service buffer should reach witness service evidence"
    );
    assert_contains(
        &witness["parallel_lowering"].to_string(),
        "send-sync",
        "parallel witness evidence should cite structured proof checks",
    );
    assert_contains(
        &witness["parallel_lowering"].to_string(),
        "accepted-lowering-gate",
        "parallel witness evidence should prove the loop crossed the accepted lowering gate",
    );
    assert_contains(
        &witness["parallel_lowering"].to_string(),
        "no-K0061-blockers",
        "parallel witness evidence should state that blocking parallel diagnostics were absent",
    );
    assert_contains(
        &witness["task_local_zones"].to_string(),
        "local_state",
        "integrated task-local witness should include the non-Send captured binding",
    );
    assert_eq!(
        witness["replay_guarantee"], "partial",
        "activity side effects must keep the integrated witness honest about exactness"
    );
    assert_eq!(
        witness["full_ecosystem_exploration"], false,
        "integrated fixture must not claim arbitrary ecosystem exploration"
    );
}

#[test]
fn integrated_negative_parallel_and_standalone_debt_are_gated() {
    let (project, _) = integration_project("v12-integrated-negative-and-debt");
    let unsafe_file = project.write(
        "src/unsafe_parallel.kobo",
        r#"
use std::rc::Rc;

fn crunch(values: Vec<u64>) {
    let state = Rc::new(1_u64);
    #[kobo::parallel]
    for value in values.iter() {
        let _seen = *value + *state;
    }
}
"#,
    );

    let unsafe_output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&unsafe_file)],
        &project.root,
    );
    assert_failure(
        &unsafe_output,
        "integrated negative fixture should reject unsafe parallel capture",
    );
    assert_contains(
        &unsafe_output.combined(),
        "non-Send",
        "unsafe parallel rejection should explain the Send/Sync blocker",
    );

    let debt_output = run_kobo(
        &[
            s("debt"),
            s("--cargo"),
            path_arg(&project.root),
            s("--json"),
        ],
        &project.root,
    );
    assert_success(
        &debt_output,
        "integrated fixture should run standalone Rust debt over the surrounding Cargo project",
    );
    let debt_json: Value =
        serde_json::from_str(&debt_output.stdout).expect("debt output should parse");
    assert_eq!(debt_json["mode"], "rust-cargo-standalone");
    assert!(
        debt_json["findings"]
            .as_array()
            .expect("debt findings should be an array")
            .iter()
            .any(|finding| finding["category"] == "external-boundary-candidate"),
        "integrated Cargo debt should report an advisory external boundary candidate: {debt_json}"
    );
}
