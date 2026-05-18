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
fn exact_witness_preserves_repeated_record_calls_from_same_crate() {
    let project = TestProject::new("v11-repeated-record-boundary-events");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "record", reason = "record gateway construction")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {
    let _first = Client::new();
    let _second = Client::new();
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
        "exact replay should support multiple recorded calls from the same crate",
    );
    let (_, witness) = first_witness(&project);
    let boundary_events = witness["events"]
        .as_array()
        .expect("events should be an array")
        .iter()
        .filter(|event| event["kind"].as_str() == Some("boundary-record"))
        .collect::<Vec<_>>();
    assert_eq!(
        boundary_events.len(),
        2,
        "both same-crate recorded calls should have independent boundary events:\n{}",
        witness
    );
    assert_ne!(
        boundary_events[0]["label"], boundary_events[1]["label"],
        "same-crate recorded calls must carry distinct call/span labels:\n{}",
        witness
    );
    assert_eq!(
        witness["ecosystem_boundaries"]
            .as_array()
            .expect("ecosystem boundaries should be an array")
            .len(),
        2,
        "witness should retain both same-crate ecosystem boundary decisions",
    );
}

#[test]
fn exact_witness_supports_imported_free_function_record_boundary() {
    assert_free_function_boundary_policy("record", "recorded-event", "boundary-record");
}

#[test]
fn exact_witness_supports_imported_free_function_model_boundary() {
    assert_free_function_boundary_policy("model", "modeled-facade", "boundary-model");
}

#[test]
fn exact_witness_supports_imported_free_function_stub_boundary() {
    assert_free_function_boundary_policy("stub", "scenario-stub", "boundary-stub");
}

#[test]
fn exact_witness_supports_nested_imported_free_function_record_boundary() {
    assert_nested_free_function_boundary_policy("record", "recorded-event", "boundary-record");
}

#[test]
fn exact_witness_supports_nested_imported_free_function_model_boundary() {
    assert_nested_free_function_boundary_policy("model", "modeled-facade", "boundary-model");
}

#[test]
fn exact_witness_supports_nested_imported_free_function_stub_boundary() {
    assert_nested_free_function_boundary_policy("stub", "scenario-stub", "boundary-stub");
}

#[test]
fn exact_witness_supports_imported_module_free_function_record_boundary() {
    assert_imported_module_free_function_boundary_policy(
        "record",
        "recorded-event",
        "boundary-record",
    );
}

#[test]
fn exact_witness_supports_imported_module_free_function_model_boundary() {
    assert_imported_module_free_function_boundary_policy(
        "model",
        "modeled-facade",
        "boundary-model",
    );
}

#[test]
fn exact_witness_supports_imported_module_free_function_stub_boundary() {
    assert_imported_module_free_function_boundary_policy("stub", "scenario-stub", "boundary-stub");
}

fn assert_free_function_boundary_policy(policy: &str, evidence: &str, event_kind: &str) {
    let project = TestProject::new(&format!("v11-free-function-{policy}-boundary"));
    let file = project.main_file(&format!(
        r#"
#[kobo::boundary(crate = "payments", policy = "{policy}", reason = "free function gateway")]
use payments::charge;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {{
    let _result = charge();
    ward.task();
}}
"#
    ));

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
        "free-function boundary should compile and replay under --engine both",
    );
    let (_, witness) = first_witness(&project);
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "payments::charge",
        "free-function boundary evidence should retain the function path",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "free_function",
        "free-function boundary evidence should expose semantic call shape",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        evidence,
        "free-function boundary evidence should use the policy-specific evidence marker",
    );
    assert_contains(
        &witness["events"].to_string(),
        event_kind,
        "generated harness and semantic trace should include the policy-specific boundary event",
    );
    assert_contains(
        &witness["events"].to_string(),
        "payments::charge@",
        "free-function boundary event should be call/span-specific",
    );
}

fn assert_nested_free_function_boundary_policy(policy: &str, evidence: &str, event_kind: &str) {
    let project = TestProject::new(&format!("v11-nested-free-function-{policy}-boundary"));
    let file = project.main_file(&format!(
        r#"
#[kobo::boundary(crate = "payments", policy = "{policy}", reason = "nested free function gateway")]
use payments::gateway::charge;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {{
    let _result = charge();
    ward.task();
}}
"#
    ));

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
        "nested free-function boundary should compile and replay under --engine both",
    );
    let (witness_path, witness) = first_witness(&project);
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "payments::gateway::charge",
        "nested free-function boundary evidence should retain the full module path",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "free_function",
        "nested free-function boundary evidence should expose semantic call shape",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        evidence,
        "nested free-function boundary evidence should use the policy-specific evidence marker",
    );
    assert_contains(
        &witness["events"].to_string(),
        event_kind,
        "generated harness and semantic trace should include the policy-specific boundary event",
    );
    assert_contains(
        &witness["events"].to_string(),
        "payments::gateway::charge@",
        "nested free-function boundary event should be call/span-specific",
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
        "nested free-function exact witness should replay with the same call-shape evidence",
    );
}

fn assert_imported_module_free_function_boundary_policy(
    policy: &str,
    evidence: &str,
    event_kind: &str,
) {
    let project = TestProject::new(&format!("v11-module-free-function-{policy}-boundary"));
    let file = project.main_file(&format!(
        r#"
#[kobo::boundary(crate = "payments", policy = "{policy}", reason = "imported module free function gateway")]
use payments::gateway;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {{
    let _result = gateway::charge();
    ward.task();
}}
"#
    ));

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
        "imported-module free-function boundary should compile and replay under --engine both",
    );
    let (witness_path, witness) = first_witness(&project);
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "payments::gateway::charge",
        "imported-module free-function evidence should retain the full module path",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "free_function",
        "imported-module free-function evidence should expose semantic call shape",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        evidence,
        "imported-module free-function evidence should use the policy-specific evidence marker",
    );
    assert_contains(
        &witness["events"].to_string(),
        event_kind,
        "generated harness and semantic trace should include the policy-specific boundary event",
    );
    assert_contains(
        &witness["events"].to_string(),
        "payments::gateway::charge@",
        "imported-module free-function boundary event should be call/span-specific",
    );
    let harness_path = witness["harness_manifest"]["harness_rs_path"]
        .as_str()
        .expect("exact witness should record generated harness path");
    let harness_source =
        fs::read_to_string(harness_path).expect("harness source should be readable");
    assert_contains(
        &harness_source,
        "pub mod gateway",
        "generated harness should model imported module free functions as nested modules",
    );
    assert_contains(
        &harness_source,
        "pub fn charge()",
        "generated harness should emit a module-level charge free function",
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
        "imported-module free-function exact witness should replay with the same call-shape evidence",
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
    assert_contains(
        &witness["events"].to_string(),
        "reqwest::Client::new",
        "recorded boundary event should identify the call path",
    );
    witness["events"][0]["label"] = serde_json::json!("reqwest");
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
        "crate-only event labels must not satisfy call-specific record evidence",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "missing recorded event evidence should use K0124",
    );
}

#[test]
fn record_boundary_event_evidence_is_call_specific() {
    let project = TestProject::new("v11-record-call-specific-evidence");
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
    assert_contains(
        &witness["events"].to_string(),
        "reqwest::Client::new",
        "recorded boundary event should identify the call path",
    );
    witness["events"][0]["label"] = serde_json::json!("reqwest");
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
        "crate-only event labels must not satisfy call-specific record evidence",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "call-specific record evidence drift should use K0124",
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
