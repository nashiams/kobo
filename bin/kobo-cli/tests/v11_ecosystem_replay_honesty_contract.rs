mod v09_common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

fn first_witness(project: &TestProject) -> (std::path::PathBuf, Value) {
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

#[test]
fn exact_witness_discloses_v11_ecosystem_scope_without_full_exploration() {
    let project = TestProject::new("v11-exact-witness-ecosystem-scope");
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
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "record boundary fixture should produce a witness");
    let (_, witness) = first_witness(&project);
    assert_eq!(witness["full_ecosystem_exploration"], false);
    assert_eq!(
        witness["replay_contract"]["full_ecosystem_exploration"],
        false
    );
    assert_contains(
        &witness.to_string(),
        "ecosystem_boundaries",
        "v0.11 witness should include explicit ecosystem boundary evidence",
    );
    assert_contains(
        &witness.to_string(),
        r#""evidence":"recorded-event""#,
        "record policy should be tied to recorded boundary event evidence",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "reqwest::Client::new",
        "boundary evidence should retain the replay-critical item path",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "source_span",
        "boundary evidence should retain source span identity",
    );
}

#[test]
fn mutated_boundary_policy_breaks_exact_replay() {
    let project = TestProject::new("v11-mutated-boundary-policy");
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
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "fixture should create an exact witness");
    let (witness_path, mut witness) = first_witness(&project);
    witness["boundary_policies"][0]["policy"] = serde_json::json!("opaque");
    witness["ecosystem_boundaries"][0]["policy"] = serde_json::json!("opaque");
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
        "exact replay must reject mutated ecosystem boundary policy evidence",
    );
    assert_contains(
        &replay.combined(),
        "K0129",
        "policy mutation should be reported as an ecosystem replay overclaim block",
    );
}

#[test]
fn record_boundary_missing_recorded_event_breaks_exact_replay() {
    let project = TestProject::new("v11-record-missing-evidence");
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
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "fixture should create an exact witness");
    let (witness_path, mut witness) = first_witness(&project);
    witness["ecosystem_boundaries"][0]["evidence"] = serde_json::json!("unverified");
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
        "record boundary without recorded event evidence must not replay as exact",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "missing recorded event evidence should use K0124",
    );
}

#[test]
fn opaque_boundary_witness_replays_as_partial_not_exact() {
    let project = TestProject::new("v11-opaque-partial-replay");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "opaque", reason = "accepted as outside exact replay")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn opaque_gateway() {
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
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &output,
        "opaque boundary should still produce a partial witness",
    );
    let (witness_path, witness) = first_witness(&project);
    assert_eq!(witness["replay_guarantee"], "partial");

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
        "partial opaque witness must not claim exact replay",
    );
    assert_contains(
        &replay.combined(),
        "opaque",
        "replay failure should disclose the opaque boundary",
    );
}

#[test]
fn configured_adapter_metadata_replays_in_canonical_boundary_evidence() {
    let project = TestProject::new("v11-adapter-boundary-replay");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"

[[ecosystem.adapter]]
crate = "reqwest"
package = "kobo-adapter-reqwest"
reason = "first-party HTTP adapter"
"#,
    );
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
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "fixture should create an exact witness");
    let (witness_path, witness) = first_witness(&project);
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "kobo-adapter-reqwest",
        "witness boundary evidence should preserve configured adapter metadata",
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
        "exact replay should preserve the same adapter evidence shape as witness generation",
    );
}
