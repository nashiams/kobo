mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo_with_timeout, s,
    CliOutput, TestProject,
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
    spawn local {
        ward.task();
    };
    ward.task();
    let request = Request { id: 1 };
    handle(request).await;
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
        "KoboHandlerOutcome",
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
        "handle",
        "boundary-record",
        "recorded-boundary-io",
        "boundary-activity",
        "activity-result",
        "ward.task.local",
        "runtime_profile",
        "runtime_profile_hash",
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
    assert_eq!(
        witness["replay_guarantee"], "partial",
        "activity side effects must keep the integrated witness honest about exactness"
    );
    assert_eq!(
        witness["full_ecosystem_exploration"], false,
        "integrated fixture must not claim arbitrary ecosystem exploration"
    );
}
