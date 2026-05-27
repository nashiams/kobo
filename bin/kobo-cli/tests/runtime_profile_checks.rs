mod cli_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_common::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const V12_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V12_TIMEOUT)
}

fn runtime_profile_project(
    label: &str,
    service_buffer: usize,
    service_backpressure: &str,
    scheduler: &str,
    record: &str,
    activity: &str,
) -> (TestProject, PathBuf) {
    let project = TestProject::new(label);
    project.write(
        "Kobo.toml",
        &format!(
            r#"[runtime.profile]
service_buffer = {service_buffer}
service_backpressure = "{service_backpressure}"
scheduler = "{scheduler}"
record = "{record}"
activity = "{activity}"
cancellation = "cooperative"
scenario_event_budget = 123
"#,
        ),
    );
    let file = project.main_file(
        r#"
struct Gateway {}

struct Request {
    id: u64,
}

struct Response {
    status: u16,
}

#[kobo::service]
impl Gateway {
    async fn submit(&self, request: Request) -> Response {
        Response { status: 202 }
    }
}

#[kobo::scenario(profile = "async")]
fn profile_path() {
    ward.task();
}
"#,
    );
    (project, file)
}

fn inspect_runtime_profile(
    service_buffer: usize,
    service_backpressure: &str,
    scheduler: &str,
    record: &str,
    activity: &str,
) -> String {
    let (project, file) = runtime_profile_project(
        "runtime-profile-inspect",
        service_buffer,
        service_backpressure,
        scheduler,
        record,
        activity,
    );
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(&output, "runtime profile fixture should inspect");
    output.combined()
}

fn first_witness(project: &TestProject) -> Value {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("runtime profile scenario should write a witness");
    serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

fn witness_runtime_profile(
    label: &str,
    service_buffer: usize,
    service_backpressure: &str,
    scheduler: &str,
    record: &str,
    activity: &str,
) -> Value {
    let (project, file) = runtime_profile_project(
        label,
        service_buffer,
        service_backpressure,
        scheduler,
        record,
        activity,
    );
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "runtime profile scenario should create a witness");
    first_witness(&project)
}

#[test]
fn runtime_profile_unifies_service_scenario_and_runtime_settings() {
    let witness = witness_runtime_profile(
        "runtime-profile-unified",
        11,
        "try-send",
        "pct-random-bounded",
        "recorded-boundary-io",
        "retry-idempotency",
    );

    assert_eq!(witness["runtime_profile"]["service"]["buffer"], 11);
    assert_eq!(
        witness["runtime_profile"]["service"]["backpressure"],
        "try-send"
    );
    assert_eq!(
        witness["runtime_profile"]["scenario"]["scheduler"],
        "pct-random-bounded"
    );
    assert_eq!(
        witness["scheduler"]["strategy"], "pct-random-bounded",
        "the configured runtime scheduler should drive witness scheduler behavior, not only profile text"
    );
    assert_eq!(
        witness["runtime_profile"]["record"]["default"],
        "recorded-boundary-io"
    );
    assert_eq!(
        witness["runtime_profile"]["activity"]["default"],
        "retry-idempotency"
    );
    assert_eq!(
        witness["runtime_profile"]["runtime"]["cancellation"],
        "cooperative"
    );
}

#[test]
fn inspect_shows_service_buffer_backpressure_and_scheduler_defaults() {
    let generated = inspect_runtime_profile(
        11,
        "try-send",
        "pct-random-bounded",
        "recorded-boundary-io",
        "retry-idempotency",
    );

    for expected in [
        "kobo: runtime profile",
        "service_buffer=11",
        "service_backpressure=try-send",
        "scheduler=pct-random-bounded",
        "record=recorded-boundary-io",
        "activity=retry-idempotency",
        "cancellation=cooperative",
    ] {
        assert_contains(
            &generated,
            expected,
            "inspect output should expose the unified runtime profile",
        );
    }
}

#[test]
fn profile_change_updates_generated_runtime_output() {
    let generated_11 = inspect_runtime_profile(
        11,
        "try-send",
        "pct-random-bounded",
        "recorded-boundary-io",
        "retry-idempotency",
    );
    let generated_23 = inspect_runtime_profile(
        23,
        "block-on-full",
        "small-random",
        "recorded-boundary-io",
        "retry-idempotency",
    );

    assert_contains(
        &generated_11,
        "tokio::sync::mpsc::channel::<GatewayMessage>(11)",
        "runtime profile service buffer should change generated channel capacity",
    );
    assert_contains(
        &generated_11,
        "self.sender.try_send(message)",
        "try-send backpressure should change generated service send behavior",
    );
    assert_contains(
        &generated_23,
        "tokio::sync::mpsc::channel::<GatewayMessage>(23)",
        "mutating profile buffer should change generated runtime output",
    );
    assert_contains(
        &generated_23,
        "self.sender.send(message).await",
        "block-on-full backpressure should retain awaited send behavior",
    );
}

#[test]
fn record_activity_defaults_are_visible_in_profile_evidence() {
    let witness = witness_runtime_profile(
        "runtime-profile-evidence",
        11,
        "try-send",
        "pct-random-bounded",
        "recorded-boundary-io",
        "retry-idempotency",
    );

    assert_contains(
        &witness["runtime_profile"].to_string(),
        "recorded-boundary-io",
        "record default should be preserved in witness evidence",
    );
    assert_contains(
        &witness["runtime_profile"].to_string(),
        "retry-idempotency",
        "activity default should be preserved in witness evidence",
    );
    assert!(
        witness["execution_digest"]["runtime_profile_hash"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "execution digest should include runtime profile material: {witness}"
    );
}

#[test]
fn profile_change_updates_evidence_digest_material() {
    let first = witness_runtime_profile(
        "runtime-profile-digest-first",
        11,
        "try-send",
        "pct-random-bounded",
        "recorded-boundary-io",
        "retry-idempotency",
    );
    let second = witness_runtime_profile(
        "runtime-profile-digest-second",
        23,
        "block-on-full",
        "small-random",
        "recorded-boundary-io",
        "retry-idempotency",
    );

    assert_ne!(
        first["runtime_profile"], second["runtime_profile"],
        "profile mutation should change structured runtime profile evidence"
    );
    assert_ne!(
        first["execution_digest"]["runtime_profile_hash"],
        second["execution_digest"]["runtime_profile_hash"],
        "profile mutation should change digest material"
    );
}
