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

fn write_activity_declaration_with_policy(project: &TestProject, retry: &str, idempotency: &str) {
    project.write(
        "mailer.kobo.d.toml",
        &format!(
            r#"schema_version = 0

[crate]
name = "mailer"
version = "0.1"
source = "bindgen"

[[activity]]
path = "mailer::send_email"
retry = "{retry}"
idempotency = "{idempotency}"
result = "record"
compensation = "cancel-email"
"#,
        ),
    );
}

fn integration_project(label: &str) -> (TestProject, PathBuf) {
    integration_project_with_activity_policy(label, "region-a", "retry-with-backoff", "message-id")
}

fn record_replay_integration_project(label: &str, config_key: &str) -> (TestProject, PathBuf) {
    let project = TestProject::new(label);
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
    let source = r#"
#[kobo::boundary(crate = "config_source", policy = "record", reason = "record service config reads")]
use config_source::config_value;

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

#[kobo::scenario(profile = "async")]
async fn record_only_replay_path() {
    let _config = config_value("__CONFIG_KEY__");
    let (client, worker) = GatewayService::start(Gateway {});
    let _submitted = client.submit(Request { id: 10 }).await.expect("submit");
    client.shutdown_and_wait(worker).await.expect("shutdown");
    let request = Request { id: 1 };
    handle(request).await;
    ward.task();
}
"#
    .replace("__CONFIG_KEY__", config_key);
    let file = project.main_file(&source);
    (project, file)
}

fn integration_project_with_activity_policy(
    label: &str,
    config_key: &str,
    retry: &str,
    idempotency: &str,
) -> (TestProject, PathBuf) {
    let project = TestProject::new(label);
    write_activity_declaration_with_policy(&project, retry, idempotency);
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
    let source = r#"
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
    let _config = config_value("__CONFIG_KEY__");
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

#[kobo::scenario(profile = "async")]
fn record_only_replay_path() {
    let _config = config_value("__CONFIG_KEY__");
    ward.task();
}
"#
    .replace("__CONFIG_KEY__", config_key);
    let file = project.main_file(&source);
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

fn first_witness_with_path(project: &TestProject) -> (PathBuf, Value) {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("integrated scenario should write a witness");
    let witness =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    (witness_path, witness)
}

fn first_witness(project: &TestProject) -> Value {
    first_witness_with_path(project).1
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
        ".par_iter()",
        ".for_each",
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
fn integrated_record_path_reuses_values_on_replay_and_changes_digest_when_mutated() {
    let (first_project, first_file) =
        record_replay_integration_project("v12-integrated-record-replay-first", "region-a");
    let first_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--target"),
            s("record_only_replay_path"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&first_file),
        ],
        &first_project.root,
    );
    assert_success(
        &first_output,
        "integrated record-only target should produce an exact witness",
    );
    let (first_witness_path, first_witness_json) = first_witness_with_path(&first_project);
    assert_eq!(
        first_witness_json["replay_guarantee"], "exact",
        "record-only integrated target should remain exact"
    );
    assert_contains(
        &first_witness_json["ecosystem_boundaries"].to_string(),
        "recorded-boundary-io",
        "integrated record-only target should carry recorded I/O evidence",
    );
    assert_not_contains(
        &first_witness_json["events"].to_string(),
        "boundary-activity",
        "record replay target should not include the activity side effect path",
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&first_witness_path),
            s("--error-format=json"),
        ],
        &first_project.root,
    );
    assert_success(
        &replay,
        "integrated record-only witness should reuse recorded values during replay",
    );

    let (second_project, second_file) =
        record_replay_integration_project("v12-integrated-record-replay-second", "region-b");
    let second_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--target"),
            s("record_only_replay_path"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&second_file),
        ],
        &second_project.root,
    );
    assert_success(
        &second_output,
        "mutated integrated record-only target should produce a witness",
    );
    let second_witness = first_witness(&second_project);

    assert_ne!(
        first_witness_json["events"], second_witness["events"],
        "mutating the integrated record value should change witness event material",
    );
    assert_ne!(
        first_witness_json["backend_replay_token"], second_witness["backend_replay_token"],
        "mutating the integrated record value should change the replay token",
    );
}

#[test]
fn integrated_activity_side_effect_is_not_rerun_by_deterministic_replay() {
    let (project, file) = integration_project("v12-integrated-activity-replay-blocked");
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
        "integrated product loop should produce a partial activity witness",
    );
    let (witness_path, witness) = first_witness_with_path(&project);
    assert_eq!(
        witness["replay_guarantee"], "partial",
        "activity side effect should keep the integrated witness partial",
    );
    assert_contains(
        &witness["events"].to_string(),
        "boundary-activity",
        "integrated product loop should record the activity boundary event",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        r#""external_internals_replayed":false"#,
        "integrated activity metadata should prove external internals were not replayed",
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(
        &replay,
        "deterministic replay must stop before rerunning integrated activity side effects",
    );
    assert_contains(
        &replay.combined(),
        "partial",
        "replay failure should disclose that activity evidence is partial",
    );
}

#[test]
fn integrated_activity_policy_mutation_changes_witness_metadata() {
    let (base_project, base_file) = integration_project("v12-integrated-activity-policy-base");
    let base_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--target"),
            s("product_loop"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&base_file),
        ],
        &base_project.root,
    );
    assert_success(
        &base_output,
        "base integrated activity policy should produce a witness",
    );
    let base_witness = first_witness(&base_project);

    let (mutated_project, mutated_file) = integration_project_with_activity_policy(
        "v12-integrated-activity-policy-mutated",
        "region-a",
        "retry-with-jitter",
        "dedupe-key",
    );
    let mutated_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--target"),
            s("product_loop"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&mutated_file),
        ],
        &mutated_project.root,
    );
    assert_success(
        &mutated_output,
        "mutated integrated activity policy should produce a witness",
    );
    let mutated_witness = first_witness(&mutated_project);

    assert_eq!(
        base_witness["replay_guarantee"], "partial",
        "base activity witness must remain partial",
    );
    assert_eq!(
        mutated_witness["replay_guarantee"], "partial",
        "mutated activity witness must remain partial",
    );
    let base_activity = activity_metadata(&base_witness);
    let mutated_activity = activity_metadata(&mutated_witness);
    assert_eq!(
        base_activity["retry"].as_str(),
        Some("retry-with-backoff"),
        "base activity retry policy should be visible in witness metadata",
    );
    assert_eq!(
        base_activity["idempotency"].as_str(),
        Some("message-id"),
        "base activity idempotency policy should be visible in witness metadata",
    );
    assert_eq!(
        mutated_activity["retry"].as_str(),
        Some("retry-with-jitter"),
        "mutated activity retry policy should reach witness metadata",
    );
    assert_eq!(
        mutated_activity["idempotency"].as_str(),
        Some("dedupe-key"),
        "mutated activity idempotency policy should reach witness metadata",
    );
    assert_ne!(
        base_activity["declaration_hash"], mutated_activity["declaration_hash"],
        "mutating activity policy should change the declaration hash evidence",
    );
    assert_contains(
        &mutated_witness["ecosystem_boundaries"].to_string(),
        r#""external_internals_replayed":false"#,
        "mutated activity policy must still keep external internals outside deterministic replay",
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

fn activity_metadata(witness: &Value) -> &Value {
    witness["ecosystem_boundaries"]
        .as_array()
        .expect("ecosystem boundaries should be an array")
        .iter()
        .find_map(|boundary| {
            (boundary["policy"].as_str() == Some("activity"))
                .then_some(&boundary["activity_metadata"])
        })
        .expect("integrated witness should contain activity metadata")
}
