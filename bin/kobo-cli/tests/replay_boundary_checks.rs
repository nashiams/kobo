mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn first_witness(project: &TestProject) -> (PathBuf, Value) {
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

fn write_activity_alias_declaration(project: &TestProject) {
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

fn record_fixture(config_key: &str, generated_seed: u64) -> String {
    format!(
        r#"
#[kobo::boundary(crate = "config_source", policy = "record", reason = "record service config reads")]
use config_source::config_value;
use config_source::generated_id;

#[kobo::scenario(profile = "async")]
fn load_config() {{
    let _config = config_value("{config_key}");
    let _id = generated_id({generated_seed});
    ward.task();
}}
"#
    )
}

#[test]
fn record_and_activity_have_first_class_policy_attributes() {
    let project = TestProject::new("service-first-class-record-activity");
    write_activity_alias_declaration(&project);
    let file = project.main_file(
        r#"
#[kobo::record(crate = "config_source", reason = "record service config reads")]
use config_source::config_value;

#[kobo::activity(crate = "mailer", reason = "email side effect runs outside replay")]
use mailer::send_email;

#[kobo::scenario(profile = "async")]
fn first_class_boundaries() {
    let _config = config_value("region-a");
    let _sent = send_email("receipt-123");
    ward.task();
}
"#,
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
    assert_success(
        &output,
        "first-class record/activity policy attributes should produce witness evidence",
    );
    let (_, witness) = first_witness(&project);
    let boundary_text = witness["ecosystem_boundaries"].to_string();
    assert_contains(
        &boundary_text,
        r#""policy":"record""#,
        "record attribute should lower to a record boundary policy",
    );
    assert_contains(
        &boundary_text,
        r#""policy":"activity""#,
        "activity attribute should lower to an activity boundary policy",
    );
}

#[test]
fn record_snapshots_value_into_kwit_and_reuses_on_replay() {
    let project = TestProject::new("service-record-boundary-reuse");
    let file = project.main_file(&record_fixture("region-a", 41));

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
    assert_success(&output, "record boundary fixture should create a witness");
    let (witness_path, witness) = first_witness(&project);
    let boundaries = witness["ecosystem_boundaries"]
        .as_array()
        .expect("ecosystem boundaries should be present");
    let record_boundaries = boundaries
        .iter()
        .filter(|boundary| boundary["policy"].as_str() == Some("record"))
        .collect::<Vec<_>>();
    assert_eq!(
        record_boundaries.len(),
        2,
        "record fixture must snapshot config and generated ID reads:\n{witness}"
    );
    for boundary in record_boundaries {
        assert_eq!(boundary["evidence"].as_str(), Some("recorded-event"));
        assert_eq!(
            boundary["capture"]["io_capture"]["mode"].as_str(),
            Some("recorded-boundary-io"),
            "record boundary should carry replayable I/O capture:\n{boundary}"
        );
        assert!(
            boundary["capture"]["io_capture"]["replay_key"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "record boundary should have a replay key:\n{boundary}"
        );
        assert_eq!(
            boundary["capture"]["external_internals_replayed"].as_bool(),
            Some(false),
            "record captures the boundary value without claiming external internals were replayed"
        );
    }
    assert_contains(
        &witness["boundary_ledger"].to_string(),
        "recorded-boundary-io",
        "the .kwit boundary ledger should store recorded I/O material",
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_success(
        &replay,
        "deterministic replay should accept the matching recorded values",
    );
}

#[test]
fn record_value_mutation_changes_witness_digest() {
    let first = TestProject::new("service-record-digest-first");
    let first_file = first.main_file(&record_fixture("region-a", 41));
    let first_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&first_file),
        ],
        &first.root,
    );
    assert_success(
        &first_output,
        "first record fixture should create a witness",
    );
    let (first_path, first_value) = first_witness(&first);

    let second = TestProject::new("service-record-digest-second");
    let second_file = second.main_file(&record_fixture("region-b", 42));
    let second_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&second_file),
        ],
        &second.root,
    );
    assert_success(
        &second_output,
        "second record fixture should create a witness",
    );
    let (second_path, second_value) = first_witness(&second);

    assert_ne!(
        first_path, second_path,
        "digest assertion must compare two different witness files"
    );
    assert_ne!(
        first_value["events"], second_value["events"],
        "changing recorded config/id values should change witness event material"
    );
    assert_ne!(
        first_value["backend_replay_token"], second_value["backend_replay_token"],
        "changing recorded values should change the witness replay token"
    );
}

#[test]
fn record_mismatch_downgrades_or_fails_exact_replay() {
    let project = TestProject::new("service-record-mismatch");
    let file = project.main_file(&record_fixture("region-a", 41));
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
    assert_success(&output, "record fixture should create an exact witness");
    let (witness_path, mut witness) = first_witness(&project);
    witness["ecosystem_boundaries"][0]["capture"]["io_capture"]["response_hash"] =
        serde_json::json!("tampered-response");
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
    assert_failure(
        &replay,
        "tampered recorded value evidence must not replay as exact",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "record mismatch should report the record evidence blocker",
    );
}

#[test]
fn activity_records_result_retry_and_idempotency_metadata() {
    let project = TestProject::new("service-activity-metadata");
    write_activity_declaration(&project);
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "mailer", policy = "activity", reason = "email side effect runs outside replay")]
use mailer::send_email;

#[kobo::scenario(profile = "async")]
fn send_receipt() {
    let _sent = send_email("receipt-123");
    ward.task();
}
"#,
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
    assert_success(&output, "activity fixture should create witness evidence");
    let (_, witness) = first_witness(&project);
    let boundary_text = witness["ecosystem_boundaries"].to_string();
    for expected in [
        r#""evidence":"activity-result""#,
        "activity-result-metadata",
        "semantic-activity-boundary",
        "activity_metadata",
        "retry-with-backoff",
        "message-id",
        r#""result":"record""#,
        "cancel-email",
        "source_span",
    ] {
        assert_contains(
            &boundary_text,
            expected,
            "activity evidence should carry result/retry/idempotency metadata",
        );
    }
    assert_contains(
        &witness["boundary_ledger"].to_string(),
        "activity-result-metadata",
        "activity result metadata should be recorded in the boundary ledger without claiming exact replay",
    );
}

#[test]
fn activity_keeps_external_effect_outside_deterministic_replay() {
    let project = TestProject::new("service-activity-outside-determinism");
    write_activity_declaration(&project);
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "mailer", policy = "activity", reason = "email side effect runs outside replay")]
use mailer::send_email;

#[kobo::scenario(profile = "async")]
fn send_receipt() {
    let _sent = send_email("receipt-123");
    ward.task();
}
"#,
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
    assert_success(&output, "activity fixture should create a partial witness");
    let (witness_path, witness) = first_witness(&project);
    assert_eq!(
        witness["replay_guarantee"], "partial",
        "activity boundaries must not claim exact deterministic replay"
    );
    assert_contains(
        &witness["events"].to_string(),
        "boundary-activity",
        "activity side effect should appear as a boundary event",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        r#""external_internals_replayed":false"#,
        "activity evidence must keep external internals outside deterministic replay",
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
        "deterministic replay should refuse to overclaim an activity witness",
    );
    assert_contains(
        &replay.combined(),
        "partial",
        "replay output should disclose the partial activity boundary",
    );
}

#[test]
fn service_diagnostic_suggests_record_activity_or_partial() {
    let project = TestProject::new("service-service-boundary-diagnostic");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
async fn send_receipt() {
    let _client = reqwest::Client::new();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "unconfigured service boundary should require a replay policy",
    );
    let combined = output.combined();
    assert_contains(
        &combined,
        "K0107",
        "diagnostic should be the boundary policy error",
    );
    assert_contains(
        &combined,
        "record it, wrap it as an activity, or keep this path partial",
        "service replay diagnostic should offer record/activity/partial choices",
    );
}

#[test]
fn opaque_boundary_cannot_claim_exact_replay() {
    let project = TestProject::new("service-opaque-boundary");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "mailer", policy = "opaque", reason = "accepted outside exact replay")]
use mailer::send_email;

#[kobo::scenario(profile = "async")]
fn send_receipt() {
    let _sent = send_email("receipt-123");
    ward.task();
}
"#,
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
    assert_success(&output, "opaque boundary should produce a partial witness");
    let (witness_path, witness) = first_witness(&project);
    assert_eq!(witness["replay_guarantee"], "partial");
    assert_contains(
        &witness["boundary_assumptions"].to_string(),
        r#""policy":"opaque""#,
        "opaque policy should be visible in boundary assumptions",
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(&replay, "opaque witness must not replay as exact");
    assert_contains(
        &replay.combined(),
        "opaque",
        "replay failure should disclose the opaque boundary",
    );
}
