mod v09_common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

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
    let project = TestProject::new("v10-recorded-http-boundary");
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
fn tokio_spawn_body_is_replayed_by_semantic_and_harness_paths() {
    let project = TestProject::new("v10-tokio-spawn-replay");
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
