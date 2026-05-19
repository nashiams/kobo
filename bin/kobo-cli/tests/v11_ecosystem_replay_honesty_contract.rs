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
const ADAPTER_FIXTURE: &str = "schema_version = 1\npackage = \"kobo-adapter-fixture\"\nversion = \"0.1.0\"\nkind = \"adapter\"\ncompatible_crate = \">=0.0.0,<999.0.0\"\nsigned_by = \"kobo-test\"\nadapter_runtime = \"kobo_adapter::Adapter\"\ncapture = \"boundary-io\"\n";
const ADAPTER_FIXTURE_SHA256: &str =
    "ffa9fbaa4f48f7b0df86b9ebb8bf9d64965d23b741260ff0d046491420fc4b0d";

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

fn write_valid_adapter_config(project: &TestProject, crate_name: &str) {
    project.write(
        ".kobo/registry/packages/adapter-fixture.toml",
        ADAPTER_FIXTURE,
    );
    project.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.adapter]]
crate = "{crate_name}"
package = "kobo-adapter-fixture"
version = "0.1.0"
source = "registry-index"
registry = "test-v0.11"
checksum = "sha256:{ADAPTER_FIXTURE_SHA256}"
compatible_crate = ">=0.0.0,<999.0.0"
metadata_path = ".kobo/registry/packages/adapter-fixture.toml"
trust_policy = "workspace-pinned"
signed_by = "kobo-test"
validated = true
adapter_runtime = "kobo_adapter::Adapter"
capture = "boundary-io"
reason = "validated test adapter"
"#
        ),
    );
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
fn backend_registry_discloses_full_depth_for_compiler_owned_and_registered_boundaries() {
    let project = TestProject::new("v11-backend-full-depth-scope");
    let output = run_kobo(&[s("sim"), s("backends"), s("--json")], &project.root);
    assert_success(&output, "backend registry should render");
    let value: Value = serde_json::from_str(&output.stdout).expect("backend JSON should parse");
    let backends = value["backends"]
        .as_array()
        .expect("backend list should be an array");

    for name in [
        "generated-rust-process",
        "loom",
        "storage-filesystem",
        "network-loopback",
    ] {
        let backend = backends
            .iter()
            .find(|backend| backend["name"] == name)
            .unwrap_or_else(|| panic!("backend `{name}` should be listed: {value}"));
        assert_eq!(
            backend["full_ecosystem_exploration"], false,
            "{name} must not claim arbitrary external crate exploration: {backend}",
        );
        assert_contains(
            &backend.to_string(),
            "registered-boundaries",
            "compiler-owned backends should disclose registered-boundary scope without arbitrary ecosystem exploration",
        );
    }

    for name in ["shuttle", "turmoil", "madsim"] {
        let backend = backends
            .iter()
            .find(|backend| backend["name"] == name)
            .unwrap_or_else(|| panic!("backend `{name}` should be listed: {value}"));
        assert_eq!(
            backend["full_ecosystem_exploration"], false,
            "{name} still must not claim arbitrary external crate exploration: {backend}",
        );
        assert_eq!(
            backend["registered_boundary_exploration"], true,
            "{name} should be full-depth for registry-validated adapter boundaries: {backend}",
        );
        assert_contains(
            &backend.to_string(),
            "full-registered-boundaries",
            "adapter backends should disclose their registry-validated production scope",
        );
    }
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
        "facade-call-capture",
        "record policy should disclose call-time boundary capture metadata",
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
    for expected in [
        "io_capture",
        "request_hash",
        "response_hash",
        "replay_key",
        "recorded-boundary-io",
        "request_body",
        "response_body",
        "status_code",
        "generated-boundary-facade-runtime",
    ] {
        assert_contains(
            &witness["ecosystem_boundaries"].to_string(),
            expected,
            "record boundaries should carry replayable boundary I/O capture, not only an event label",
        );
    }
    let boundary = witness["ecosystem_boundaries"]
        .as_array()
        .and_then(|boundaries| {
            boundaries
                .iter()
                .find(|boundary| boundary["policy"].as_str() == Some("record"))
        })
        .expect("record boundary should be present");
    let io_capture = &boundary["capture"]["io_capture"];
    assert_eq!(
        io_capture["request"]["payload"]["call_path"].as_str(),
        Some("reqwest::Client::new"),
        "recorded I/O capture should store the concrete boundary request path"
    );
    assert_eq!(
        io_capture["response"]["payload"]["status"].as_str(),
        Some("recorded"),
        "recorded I/O capture should store a replayable response payload"
    );
    assert!(
        io_capture["response"]["payload"]["replay_result"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "recorded I/O capture should include a deterministic replay result"
    );
    let record_event = witness["events"]
        .as_array()
        .and_then(|events| {
            events
                .iter()
                .find(|event| event["kind"].as_str() == Some("boundary-record"))
        })
        .expect("record boundary event should be present");
    assert_eq!(
        record_event["io_capture"], *io_capture,
        "the replay-critical event stream should carry the same captured boundary I/O as ecosystem boundary evidence"
    );
    let ledger_entry = witness["boundary_ledger"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["policy"].as_str() == Some("record"))
        })
        .expect("record boundary ledger entry should be present");
    assert_eq!(
        ledger_entry["io_capture"], *io_capture,
        "the boundary ledger should carry the same captured boundary I/O as the event stream"
    );
}

#[test]
fn activity_witness_carries_retry_idempotency_and_result_metadata() {
    let project = TestProject::new("v11-activity-witness-metadata");
    project.write(
        "reqwest.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "reqwest"
version = "0.1"
source = "bindgen"

[[activity]]
path = "reqwest::Client::new"
retry = "retry-safe"
idempotency = "request-id"
result = "record"
compensation = "cancel-request"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "activity", reason = "external request activity")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn request_activity() {
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
        "activity boundary with declaration metadata should produce witness evidence",
    );
    let (_, witness) = first_witness(&project);
    let boundary_text = witness["ecosystem_boundaries"].to_string();
    for expected in [
        r#""evidence":"activity-result""#,
        "activity_metadata",
        "retry-safe",
        "request-id",
        "record",
        "cancel-request",
        "boundary-call-capture",
    ] {
        assert_contains(
            &boundary_text,
            expected,
            "activity witness should carry production-depth activity evidence",
        );
    }
}

#[test]
fn test_sim_rejects_activity_boundary_without_complete_metadata() {
    let project = TestProject::new("v11-activity-witness-incomplete-metadata");
    project.write(
        "reqwest.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "reqwest"
version = "0.1"
source = "bindgen"

[[activity]]
path = "reqwest::Client::new"
retry = "retry-safe"
idempotency = "request-id"
result = "record"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "activity", reason = "external request activity")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn request_activity() {
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

    assert_failure(
        &output,
        "test --sim must reject activity evidence without complete retry/idempotency/result/compensation metadata",
    );
    assert_contains(
        &output.combined(),
        "K0125",
        "incomplete activity metadata should use the v0.11 activity diagnostic",
    );
    assert!(
        project.find_files_with_ext("kwit").is_empty(),
        "invalid activity metadata must not produce .kwit witnesses"
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
fn record_boundary_captures_facade_call_arguments_and_return_payload() {
    let project = TestProject::new("v11-record-boundary-real-io-arguments");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "payments", policy = "record", reason = "capture charge request")]
use payments::charge;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {
    let _result = charge("acct_123", 42);
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
        "record boundary with call arguments should produce an exact witness",
    );
    let (_, witness) = first_witness(&project);
    let boundary = witness["ecosystem_boundaries"]
        .as_array()
        .and_then(|boundaries| {
            boundaries
                .iter()
                .find(|boundary| boundary["policy"].as_str() == Some("record"))
        })
        .expect("record boundary should be present");
    let io_payload = &boundary["capture"]["io_capture"];
    let request = io_payload["request"]["payload"].to_string();
    for expected in [
        "argument_count",
        "argument_0_source",
        "\\\"acct_123\\\"",
        "argument_1_source",
        "42",
        "argument_0_type",
        "argument_1_type",
        "request_body",
    ] {
        assert_contains(
            &request,
            expected,
            "record boundary request payload should be captured by the generated facade call",
        );
    }
    let response = io_payload["response"]["payload"].to_string();
    for expected in ["return_payload", "response_body", "facade_return"] {
        assert_contains(
            &response,
            expected,
            "record boundary response payload should include the generated facade return",
        );
    }
    assert_eq!(
        witness["boundary_ledger"][0]["io_capture"], boundary["capture"]["io_capture"],
        "boundary ledger should carry the same facade-captured I/O payload"
    );
}

#[test]
fn record_boundary_captures_external_method_call_arguments_and_return_payload() {
    let project = TestProject::new("v11-record-boundary-method-io-arguments");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "record", reason = "capture HTTP request")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {
    let client = Client::new();
    let request = client.get("https://example.test/api");
    let _response = request.send();
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
        "record boundary method call should produce an exact witness",
    );
    let (_, witness) = first_witness(&project);
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "reqwest::Client::get",
        "method call boundary should reach ecosystem boundary evidence",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "method",
        "method call boundary should preserve call shape",
    );
    assert_contains(
        &witness["boundary_ledger"].to_string(),
        "reqwest::Client::get",
        "method call boundary should reach the boundary ledger",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "reqwest::RequestBuilder::send",
        "method return values should keep enough type identity for chained boundary calls",
    );
    for expected in [
        "argument_0_source",
        "https://example.test/api",
        "argument_0_type",
        "return_payload",
        "facade_return:reqwest::RequestBuilder",
        "facade_return:reqwest::Response",
    ] {
        assert_contains(
            &witness["ecosystem_boundaries"].to_string(),
            expected,
            "method record boundary should capture request arguments and facade return payload",
        );
    }
}

#[test]
fn record_boundary_captures_external_method_call_inside_helper() {
    let project = TestProject::new("v11-record-boundary-helper-method");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "record", reason = "capture helper HTTP request")]
use reqwest::Client;

fn send_request(client: Client) {
    let _request = client.get("https://example.test/helper");
}

#[kobo::scenario(profile = "async")]
fn recorded_gateway() {
    let client = Client::new();
    send_request(client);
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
        "helper method boundary should produce an exact witness",
    );
    let (_, witness) = first_witness(&project);
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "reqwest::Client::get",
        "helper-local method call should retain external boundary value metadata",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        "https://example.test/helper",
        "helper-local method call should retain captured argument payload",
    );
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
    if policy == "model" {
        write_valid_adapter_config(&project, "payments");
    }
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
    if policy == "model" {
        write_valid_adapter_config(&project, "payments");
    }
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
    if policy == "model" {
        write_valid_adapter_config(&project, "payments");
    }
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
fn record_boundary_missing_io_capture_breaks_exact_replay() {
    let project = TestProject::new("v11-record-missing-io-capture");
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
    witness["ecosystem_boundaries"][0]["capture"]["io_capture"] = serde_json::Value::Null;
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
        "record replay must reject witnesses with missing boundary I/O capture",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "missing record I/O capture should use K0124",
    );
}

#[test]
fn record_boundary_tampered_io_capture_hash_breaks_exact_replay() {
    let project = TestProject::new("v11-record-tampered-io-capture");
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
    witness["ecosystem_boundaries"][0]["capture"]["io_capture"]["request_hash"] =
        serde_json::json!("tampered");
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
        "record replay must reject witnesses with modified boundary I/O hashes",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "tampered record I/O capture should use K0124",
    );
}

#[test]
fn record_boundary_tampered_ledger_io_capture_breaks_exact_replay() {
    let project = TestProject::new("v11-record-tampered-ledger-io-capture");
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
    witness["boundary_ledger"][0]["io_capture"]["response_hash"] =
        serde_json::json!("ledger-tampered");
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
        "record replay must reject witnesses whose boundary ledger I/O capture diverges",
    );
    assert_contains(
        &replay.combined(),
        "K0124",
        "tampered boundary ledger I/O capture should use K0124",
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
    write_valid_adapter_config(&project, "reqwest");
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
        "kobo-adapter-fixture",
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

#[test]
fn unvalidated_adapter_metadata_rejects_exact_witness_generation() {
    let project = TestProject::new("v11-unvalidated-adapter-boundary");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"

[[ecosystem.adapter]]
crate = "reqwest"
package = "kobo-adapter-reqwest"
reason = "unvalidated adapter should not support exact replay"
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

    assert_failure(
        &output,
        "test --sim must reject unvalidated adapter metadata before writing exact witness evidence",
    );
    assert_contains(
        &output.combined(),
        "K0123",
        "unvalidated adapter metadata should use the adapter package diagnostic",
    );
}
