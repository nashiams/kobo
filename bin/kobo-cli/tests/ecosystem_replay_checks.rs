mod cli_test_support;

use std::fs;
use std::path::Path;
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const ECOSYSTEM_REPLAY_TIMEOUT: Duration = Duration::from_secs(30);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, ECOSYSTEM_REPLAY_TIMEOUT)
}

fn read_first_witness(project: &TestProject) -> (std::path::PathBuf, Value) {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    (witness_path, witness)
}

fn replay_exact(project_root: &Path, witness_path: &Path) {
    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(witness_path),
            s("--error-format=json"),
        ],
        project_root,
    );
    assert_success(&replay, "exact witness should replay");
    assert_contains(
        &replay.combined(),
        r#""replay":"exact""#,
        "replay output should confirm exact replay",
    );
}

#[test]
fn recorded_http_boundary_is_exact_replay_evidence() {
    let project = TestProject::new("runtime-recorded-http-boundary");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "record", reason = "record gateway construction")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {
    let _client = Client::new();
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("43"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "record policy should make the ecosystem boundary replayable",
    );
    let (witness_path, witness) = read_first_witness(&project);
    assert_eq!(witness["replay_guarantee"], "exact");
    assert_eq!(witness["full_ecosystem_exploration"], false);
    assert_eq!(
        witness["replay_contract"]["scope"],
        "generated-user-rust-adapter"
    );
    assert_eq!(
        witness["replay_contract"]["full_ecosystem_exploration"],
        false
    );
    assert_contains(
        &witness["replay_contract"]["facades"].to_string(),
        "external-boundary-record-facade:reqwest",
        "record policy should be explicit facade-scoped evidence, not arbitrary reqwest execution",
    );
    assert_eq!(
        witness["harness_manifest"]["full_ecosystem_exploration"],
        false
    );
    assert_contains(
        &witness["boundary_policies"].to_string(),
        r#""policy":"record""#,
        "witness should preserve the source boundary policy",
    );
    assert_contains(
        &witness["events"].to_string(),
        "boundary-record",
        "recorded ecosystem boundary should appear in the replay event stream",
    );
    replay_exact(&project.root, &witness_path);
}

#[test]
fn replay_rejects_exact_witness_that_claims_full_ecosystem_exploration() {
    let project = TestProject::new("runtime-full-ecosystem-overclaim");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn generated_scope_only() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("44"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &output,
        "fixture should create an exact compiler-owned witness",
    );
    let (witness_path, mut witness) = read_first_witness(&project);
    witness["full_ecosystem_exploration"] = serde_json::json!(true);
    witness["replay_contract"]["full_ecosystem_exploration"] = serde_json::json!(true);
    witness["harness_manifest"]["full_ecosystem_exploration"] = serde_json::json!(true);
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(&witness).expect("witness should serialize"),
    )
    .expect("witness should write");

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert!(
        !replay.status.success(),
        "replay must reject exact witnesses that overclaim ecosystem coverage: {}",
        replay.combined()
    );
    assert_contains(
        &replay.combined(),
        "K0117",
        "overclaimed full ecosystem scope should be a replay check error",
    );
}

#[test]
fn tokio_spawn_body_is_replayed_by_semantic_and_harness_paths() {
    let project = TestProject::new("runtime-tokio-spawn-replay");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn spawned_reply() {
    let reply = ReplyToken {};
    tokio::spawn(async move {
        reply.reply();
    });
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("47"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "spawned async body should discharge the obligation under replay",
    );
    let (witness_path, witness) = read_first_witness(&project);
    assert_eq!(witness["replay_guarantee"], "exact");
    assert_eq!(witness["full_ecosystem_exploration"], false);
    assert_eq!(
        witness["replay_contract"]["scope"],
        "generated-user-rust-adapter"
    );
    assert_contains(
        &witness["replay_contract"]["facades"].to_string(),
        "tokio-spawn-facade",
        "Tokio-shaped replay should disclose the generated Tokio facade",
    );
    assert_contains(
        &witness["events"].to_string(),
        "deterministic-task",
        "spawn should be represented as scheduler evidence",
    );
    assert_contains(
        &witness["function_summaries"].to_string(),
        "reply",
        "spawn body discharge should be present in semantic summaries",
    );
    replay_exact(&project.root, &witness_path);
}

#[test]
fn cancellation_scheduler_history_is_visible_in_service_foundation_witness() {
    let project = TestProject::new("runtime-service-foundation-cancel-history");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn cancellable_gateway() {
    let reply = ReplyToken {};
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("51"),
            s("--inject"),
            s("cancel"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert!(
        !output.status.success(),
        "cancel injection should preserve a failure witness for service shutdown review: {}",
        output.combined()
    );
    let (_, witness) = read_first_witness(&project);
    let scheduler = &witness["scheduler"];
    assert_eq!(
        scheduler["cancellation"]["mode"], "explicit-scheduler-history",
        "service work needs cancellation to be a replayable scheduler history, not an unsourced status label:\n{}",
        witness
    );
    assert_eq!(
        scheduler["cancellation"]["token_source"], "kobo.scheduler.cancel",
        "service shutdown should inherit a named cancellation token facade from runtime:\n{}",
        witness
    );
    let cancellation_events = scheduler["cancellation"]["events"].to_string();
    for expected in [
        "scheduler-cancel-path",
        "scheduler-future-dropped",
        "failure-injection-cancel",
    ] {
        assert_contains(
            &cancellation_events,
            expected,
            "cancellation scheduler evidence should name every replay-critical event",
        );
    }
}
