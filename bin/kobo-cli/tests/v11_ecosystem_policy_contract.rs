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
fn kobo_toml_ecosystem_policy_is_loaded_for_replay_critical_check() {
    let project = TestProject::new("v11-ecosystem-policy-config");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"
replay_unknown = "debt"

[[ecosystem.crate]]
name = "reqwest"
policy = "record"
reason = "record gateway construction"
"#,
    );
    let file = project.main_file(
        r#"
use reqwest::Client;

fn main() {
    let _client = Client::new();
}
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "explicit project policy evidence should be visible without blocking normal replay-critical check",
    );
    assert_contains(
        &output.combined(),
        r#""policy":"record""#,
        "Kobo.toml crate policy must be loaded into replay-critical boundary output",
    );
    assert_contains(
        &output.combined(),
        "record gateway construction",
        "policy reason must be preserved",
    );
}

#[test]
fn source_boundary_overrides_default_policy_with_reason() {
    let project = TestProject::new("v11-source-boundary-overrides");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "debt"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "opaque", reason = "accepted outside modeled island")]
use reqwest::Client;

fn main() {
    let _client = Client::new();
}
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "explicit source policy evidence should be visible without blocking normal replay-critical check",
    );
    assert_contains(
        &output.combined(),
        r#""policy":"opaque""#,
        "source boundary policy must override the project default",
    );
    assert_contains(
        &output.combined(),
        "accepted outside modeled island",
        "source boundary reason must be preserved",
    );
}

#[test]
fn invalid_ecosystem_policy_emits_k0120() {
    let project = TestProject::new("v11-invalid-ecosystem-policy");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "magic"
"#,
    );
    let file = project.main_file("fn main() {}\n");

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(&output, "invalid ecosystem policy must fail");
    assert_contains(
        &output.combined(),
        "K0120",
        "invalid policy should use the v0.11 ecosystem parse diagnostic",
    );
}

#[test]
fn model_boundary_without_adapter_emits_k0123() {
    let project = TestProject::new("v11-model-missing-adapter");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "payments", policy = "model", reason = "modeled facade required")]
use payments::charge;

fn main() {
    let _ = charge();
}
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(&output, "model boundary without adapter must fail");
    assert_contains(
        &output.combined(),
        "K0123",
        "missing model adapter should use K0123",
    );
}

#[test]
fn typed_boundary_without_declaration_is_visible_debt_not_normal_build_blocker() {
    let project = TestProject::new("v11-typed-missing-declaration");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "sqlx", policy = "typed", reason = "needs ownership declarations")]
use sqlx::Client;

fn main() {
    let _client = Client::new();
}
"#,
    );

    let normal = run_kobo(&[s("check"), path_arg(&file)], &project.root);
    assert_success(
        &normal,
        "normal check path must keep unknown external crates usable without Kobo metadata",
    );

    let replay_critical = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &replay_critical,
        "typed replay-critical boundary should require a declaration file",
    );
    assert_contains(
        &replay_critical.combined(),
        "K0122",
        "typed boundary without declaration must use the v0.11 missing-declaration diagnostic",
    );
}

#[test]
fn activity_boundary_records_result_but_not_external_internals() {
    let project = TestProject::new("v11-activity-boundary");
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "sqlx", policy = "activity", reason = "database work runs outside deterministic replay")]
use sqlx::Client;

#[kobo::scenario(profile = "async")]
fn activity_gateway() {
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
        "activity boundary should produce replay evidence without claiming external internals",
    );
    let (_, witness) = first_witness(&project);
    assert_eq!(witness["replay_guarantee"], "partial");
    assert_contains(
        &witness["events"].to_string(),
        "boundary-activity",
        "activity boundary must record result metadata as event evidence",
    );
    assert_contains(
        &witness["boundary_assumptions"].to_string(),
        "database work runs outside deterministic replay",
        "activity reason must remain visible in witness assumptions",
    );
}
