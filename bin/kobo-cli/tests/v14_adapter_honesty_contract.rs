mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_failure, assert_success, first_json, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V14_TIMEOUT: Duration = Duration::from_secs(60);
const ADAPTER_FIXTURE: &str = "schema_version = 1\npackage = \"kobo-adapter-fixture\"\nversion = \"1.0.0\"\nkind = \"adapter\"\ncompatible_crate = \">=0.0.0,<999.0.0\"\nsigned_by = \"kobo-test\"\nadapter_runtime = \"kobo_adapter::Adapter\"\ncapture = \"boundary-io\"\n";
const ADAPTER_FIXTURE_SHA256: &str =
    "8f2cd8da1c4909d26f5324c75ae00eaea2e5e691a5f99b0b38f0850417dfdf04";

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V14_TIMEOUT)
}

#[derive(Clone, Copy)]
struct AdapterSpec<'a> {
    crate_name: &'a str,
    confidence: &'a str,
    version: &'a str,
    policy: &'a str,
}

fn write_adapter_config(project: &TestProject, specs: &[AdapterSpec<'_>]) {
    project.write(
        ".kobo/registry/packages/adapter-fixture.toml",
        ADAPTER_FIXTURE,
    );
    let mut config = String::from("[ecosystem]\ndefault = \"opaque\"\n\n");
    for spec in specs {
        config.push_str(&format!(
            r#"[[ecosystem.adapter]]
crate = "{crate_name}"
package = "kobo-adapter-fixture"
version = "{version}"
source = "registry-index"
registry = "test-v0.14"
checksum = "sha256:{ADAPTER_FIXTURE_SHA256}"
compatible_crate = ">=0.0.0,<999.0.0"
metadata_path = ".kobo/registry/packages/adapter-fixture.toml"
trust_policy = "workspace-pinned"
signed_by = "kobo-test"
validated = true
adapter_runtime = "kobo_adapter::Adapter"
capture = "boundary-io"
confidence = "{confidence}"
reason = "{crate_name} {confidence} adapter"

[[ecosystem.crate]]
name = "{crate_name}"
policy = "{policy}"
reason = "{crate_name} boundary policy"

"#,
            crate_name = spec.crate_name,
            confidence = spec.confidence,
            version = spec.version,
            policy = spec.policy,
        ));
    }
    project.write("Kobo.toml", &config);
}

fn adapter_source(target: &str, specs: &[AdapterSpec<'_>]) -> String {
    let imports = specs
        .iter()
        .map(|spec| {
            format!(
                r#"#[kobo::boundary(crate = "{crate_name}", policy = "{policy}", reason = "{crate_name} adapter boundary")]
use {crate_name}::call as {crate_name}_call;
"#,
                crate_name = spec.crate_name,
                policy = spec.policy,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let calls = specs
        .iter()
        .map(|spec| format!("    let _{} = {}_call();", spec.crate_name, spec.crate_name))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"
{imports}

#[kobo::scenario(profile = "sync")]
fn {target}() {{
{calls}
}}
"#
    )
}

fn debt_source() -> &'static str {
    r#"
#[kobo::boundary(crate = "payments", policy = "debt", reason = "unverified adapter")]
use payments::charge;

#[kobo::scenario(profile = "sync")]
fn debt_boundary_case() {
    let _result = charge();
}
"#
}

fn unconfigured_boundary_source(policy: &str, target: &str) -> String {
    format!(
        r#"
#[kobo::boundary(crate = "payments", policy = "{policy}", reason = "review fixture")]
use payments::charge;

#[kobo::scenario(profile = "sync")]
fn {target}() {{
    let _result = charge();
}}
"#
    )
}

fn emit_artifact(project: &TestProject, source: &str, target: &str, replay_grade: &str) -> PathBuf {
    let file = project.main_file(source);
    let artifact_path = project.root.join(format!("{target}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s(target),
            s("--output"),
            path_arg(&artifact_path),
            s("--replay-grade"),
            s(replay_grade),
        ],
        &project.root,
    );
    assert_success(&output, "proof emit should produce a valid artifact");
    artifact_path
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("json should read"))
        .expect("json should parse")
}

fn adapter_confidences(artifact: &Value) -> Vec<String> {
    artifact["adapter_confidence"]
        .as_array()
        .expect("adapter_confidence should be an array")
        .iter()
        .map(|entry| {
            entry["confidence"]
                .as_str()
                .expect("confidence should be a string")
                .to_owned()
        })
        .collect()
}

fn first_kwit_path(project: &TestProject) -> PathBuf {
    project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist")
}

fn first_kwit_proof_path(project: &TestProject) -> PathBuf {
    project
        .find_files_with_ext("json")
        .into_iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".kwit.proof.json"))
        })
        .expect("proof artifact should be emitted next to the witness")
}

#[test]
fn adapter_confidence_exact_modeled_sampled_metadata_only_are_serialized() {
    let project = TestProject::new("v14-adapter-confidence-all");
    let specs = [
        AdapterSpec {
            crate_name: "exactpay",
            confidence: "exact",
            version: "1.0.0",
            policy: "record",
        },
        AdapterSpec {
            crate_name: "modeledpay",
            confidence: "modeled",
            version: "1.0.0",
            policy: "model",
        },
        AdapterSpec {
            crate_name: "samplepay",
            confidence: "sampled",
            version: "1.0.0",
            policy: "record",
        },
        AdapterSpec {
            crate_name: "metapay",
            confidence: "metadata-only",
            version: "1.0.0",
            policy: "typed",
        },
    ];
    write_adapter_config(&project, &specs);
    let artifact_path = emit_artifact(
        &project,
        &adapter_source("all_adapter_confidence_case", &specs),
        "all_adapter_confidence_case",
        "partial",
    );
    let artifact = read_json(&artifact_path);

    let mut observed = adapter_confidences(&artifact);
    observed.sort();
    assert_eq!(observed, ["exact", "metadata-only", "modeled", "sampled"]);
}

#[test]
fn sampled_adapter_emits_probing_pass_not_proof() {
    let project = TestProject::new("v14-sampled-adapter");
    let specs = [AdapterSpec {
        crate_name: "samplepay",
        confidence: "sampled",
        version: "1.0.0",
        policy: "record",
    }];
    write_adapter_config(&project, &specs);
    let artifact_path = emit_artifact(
        &project,
        &adapter_source("sampled_adapter_case", &specs),
        "sampled_adapter_case",
        "exact",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["replay_grade"], "not_replayable");
    assert_eq!(
        artifact["adapter_confidence"][0]["outcome"], "probing_pass",
        "sampled adapters should produce probing evidence, not proof evidence: {artifact}"
    );
}

#[test]
fn metadata_only_adapter_cannot_claim_replayable_behavior() {
    let project = TestProject::new("v14-metadata-adapter");
    let specs = [AdapterSpec {
        crate_name: "metapay",
        confidence: "metadata-only",
        version: "1.0.0",
        policy: "typed",
    }];
    write_adapter_config(&project, &specs);
    let artifact_path = emit_artifact(
        &project,
        &adapter_source("metadata_adapter_case", &specs),
        "metadata_adapter_case",
        "exact",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["replay_grade"], "not_replayable");
    assert_eq!(
        artifact["adapter_confidence"][0]["confidence"],
        "metadata-only"
    );
}

#[test]
fn stale_adapter_downgrades_to_debt_or_not_replayable() {
    let project = TestProject::new("v14-stale-adapter");
    let specs = [AdapterSpec {
        crate_name: "stale_pay",
        confidence: "exact",
        version: "0.0.0-stale",
        policy: "record",
    }];
    write_adapter_config(&project, &specs);
    let artifact_path = emit_artifact(
        &project,
        &adapter_source("stale_adapter_case", &specs),
        "stale_adapter_case",
        "exact",
    );
    let artifact = read_json(&artifact_path);

    assert!(
        matches!(
            artifact["replay_grade"].as_str(),
            Some("debt" | "not_replayable")
        ),
        "stale adapters must downgrade replay grade: {artifact}"
    );
    assert_eq!(artifact["adapter_confidence"][0]["outcome"], "debt");
}

#[test]
fn exact_replay_rejected_through_policy_gap() {
    let project = TestProject::new("v14-exact-policy-gap");
    let file = project.main_file(debt_source());
    let artifact_path = project.root.join("debt_boundary_case.kproof");
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s("debt_boundary_case"),
            s("--output"),
            path_arg(&artifact_path),
            s("--replay-grade"),
            s("exact"),
        ],
        &project.root,
    );

    assert_failure(&output, "exact proof over debt boundary should fail");
    assert!(
        output
            .combined()
            .contains("exact replay crosses disallowed boundary"),
        "failure should name the policy gap: {}",
        output.combined()
    );
}

#[test]
fn exact_replay_rejected_through_stub_boundary() {
    let project = TestProject::new("v14-exact-stub-boundary");
    let file = project.main_file(&unconfigured_boundary_source("stub", "stub_boundary_case"));
    let artifact_path = project.root.join("stub_boundary_case.kproof");
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s("stub_boundary_case"),
            s("--output"),
            path_arg(&artifact_path),
            s("--replay-grade"),
            s("exact"),
        ],
        &project.root,
    );

    assert_failure(&output, "exact proof over stub boundary should fail");
    assert!(
        output
            .combined()
            .contains("exact replay crosses disallowed boundary"),
        "failure should name stub as a policy gap: {}",
        output.combined()
    );
}

#[test]
fn exact_replay_rejected_when_record_boundary_lacks_adapter_evidence() {
    let project = TestProject::new("v14-exact-unsupported-adapter");
    let file = project.main_file(&unconfigured_boundary_source(
        "record",
        "unsupported_adapter_case",
    ));
    let artifact_path = project.root.join("unsupported_adapter_case.kproof");
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s("unsupported_adapter_case"),
            s("--output"),
            path_arg(&artifact_path),
            s("--replay-grade"),
            s("exact"),
        ],
        &project.root,
    );

    assert_failure(&output, "exact proof without adapter evidence should fail");
    assert!(
        output.combined().contains("unsupported adapter boundary"),
        "failure should name missing adapter evidence: {}",
        output.combined()
    );
}

#[test]
fn replay_grade_and_adapter_confidence_visible_in_kwit_kproof_inspect_and_json() {
    let project = TestProject::new("v14-adapter-visibility");
    let specs = [AdapterSpec {
        crate_name: "exactpay",
        confidence: "exact",
        version: "1.0.0",
        policy: "record",
    }];
    write_adapter_config(&project, &specs);
    let source = adapter_source("adapter_visibility_case", &specs);
    let file = project.main_file(&source);

    let test_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1406"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s("adapter_visibility_case"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&test_output, "test --sim should emit a witness");
    let witness = read_json(&first_kwit_path(&project));
    assert!(
        witness["replay_grade"].as_str().is_some(),
        ".kwit should expose replay grade: {witness}"
    );
    assert_eq!(
        witness["adapter_confidence"][0]["confidence"], "exact",
        ".kwit should expose adapter confidence: {witness}"
    );

    let kwit_proof = read_json(&first_kwit_proof_path(&project));
    assert_eq!(kwit_proof["adapter_confidence"][0]["confidence"], "exact");

    let inspect = run_kobo(&[s("inspect"), s("--sim"), path_arg(&file)], &project.root);
    assert_success(&inspect, "inspect --sim should succeed");
    assert!(
        inspect.combined().contains("adapter_confidence=exact"),
        "inspect should expose adapter confidence: {}",
        inspect.combined()
    );

    let json_output = run_kobo(
        &[
            s("proof"),
            s("verify"),
            s("--json"),
            path_arg(&first_kwit_proof_path(&project)),
        ],
        &project.root,
    );
    assert_success(&json_output, "proof verify --json should succeed");
    let json = first_json(&json_output, "proof verify json");
    assert!(
        json["replay_grade"].as_str().is_some(),
        "proof verify JSON should expose replay grade: {json}"
    );
    assert_eq!(json["adapter_confidence"][0]["confidence"], "exact");
}
