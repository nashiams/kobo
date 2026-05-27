mod cli_common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use cli_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};
use serde_json::Value;

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
    let project = TestProject::new("ecosystem-ecosystem-policy-config");
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
    let project = TestProject::new("ecosystem-source-boundary-overrides");
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
fn inspect_projects_boundary_policy_evidence() {
    let project = TestProject::new("ecosystem-inspect-boundary-policy");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"

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

    let inspect = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(
        &inspect,
        "inspect should project configured boundary policy evidence",
    );
    assert_contains(
        &inspect.combined(),
        "kobo-boundary-policy",
        "inspect output should expose boundary policy metadata",
    );
    assert_contains(
        &inspect.combined(),
        "crate=reqwest policy=record source=project-crate",
        "inspect output should preserve the configured crate policy source",
    );
    assert_contains(
        &inspect.combined(),
        "call=reqwest::Client::new",
        "inspect output should preserve the resolved boundary call path",
    );
    assert_contains(
        &inspect.combined(),
        "span=",
        "inspect output should preserve source span evidence for the boundary call",
    );
    assert_contains(
        &inspect.combined(),
        "record gateway construction",
        "inspect output should preserve the policy reason",
    );

    let inspect_sim = run_kobo(&[s("inspect"), path_arg(&file), s("--sim")], &project.root);
    assert_success(
        &inspect_sim,
        "inspect --sim should project configured boundary policy evidence",
    );
    assert_contains(
        &inspect_sim.combined(),
        "kobo-boundary-policy",
        "inspect --sim output should expose boundary policy metadata",
    );
    assert_contains(
        &inspect_sim.combined(),
        "crate=reqwest policy=record source=project-crate",
        "inspect --sim output should preserve the configured crate policy source",
    );
    assert_contains(
        &inspect_sim.combined(),
        "call=reqwest::Client::new",
        "inspect --sim output should preserve the resolved boundary call path",
    );
}

#[test]
fn debt_projects_boundary_policy_evidence() {
    let project = TestProject::new("ecosystem-debt-boundary-policy");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"

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

    let debt = run_kobo(&[s("debt"), path_arg(&file)], &project.root);
    assert_success(
        &debt,
        "debt should project configured boundary policy evidence",
    );
    assert_contains(
        &debt.combined(),
        "Boundary policy debt",
        "human debt output should expose boundary policy debt",
    );
    assert_contains(
        &debt.combined(),
        "crate=reqwest policy=record source=project-crate",
        "human debt output should preserve the configured crate policy source",
    );
    assert_contains(
        &debt.combined(),
        "call=reqwest::Client::new",
        "human debt output should preserve the resolved boundary call path",
    );
    assert_contains(
        &debt.combined(),
        "span=",
        "human debt output should preserve source span evidence for the boundary call",
    );
    assert_contains(
        &debt.combined(),
        "record gateway construction",
        "human debt output should preserve the policy reason",
    );

    let json = run_kobo(&[s("debt"), path_arg(&file), s("--json")], &project.root);
    assert_success(
        &json,
        "debt --json should project configured boundary policy evidence",
    );
    assert_contains(
        &json.combined(),
        r#""boundary_policies""#,
        "JSON debt output should include boundary policy evidence",
    );
    assert_contains(
        &json.combined(),
        r#""policy": "record""#,
        "JSON debt output should preserve the configured policy",
    );
    assert_contains(
        &json.combined(),
        r#""source": "project-crate""#,
        "JSON debt output should preserve the configured policy source",
    );
    assert_contains(
        &json.combined(),
        r#""reqwest::Client::new""#,
        "JSON debt output should preserve the resolved boundary call path",
    );
    assert_contains(
        &json.combined(),
        r#""spans""#,
        "JSON debt output should preserve structured boundary source spans",
    );
}

#[test]
fn boundary_policy_spans_are_call_site_anchored() {
    let project = TestProject::new("ecosystem-boundary-policy-call-site-spans");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"

[[ecosystem.crate]]
name = "reqwest"
policy = "record"
reason = "record gateway construction"
"#,
    );
    let source = r#"
use reqwest::Client;

fn main() {
    // Client::new in a comment must not become span evidence.
    let _text = "Client::new in a string must not become span evidence";
    let _first = Client::new();
    let _second = Client::new();
}
"#;
    let file = project.main_file(source);

    let json = run_kobo(&[s("debt"), path_arg(&file), s("--json")], &project.root);
    assert_success(
        &json,
        "debt --json should emit boundary call-site span evidence",
    );
    let parsed: Value = serde_json::from_str(&json.stdout).expect("debt output should be JSON");
    let boundary = parsed["boundary_policies"]
        .as_array()
        .expect("boundary policies should be an array")
        .iter()
        .find(|entry| entry["crate"] == "reqwest" && entry["policy"] == "record")
        .expect("reqwest record boundary policy should exist");
    let spans = boundary["spans"]
        .as_array()
        .expect("boundary spans should be an array");
    assert!(
        spans.len() >= 2,
        "repeated call sites should produce distinct span evidence:\n{}",
        boundary
    );
    let comment_start = source
        .find("// Client::new")
        .expect("comment marker should exist");
    let first_call = source
        .find("let _first = Client::new")
        .expect("first call should exist");
    for span in spans {
        let start = span["start"]
            .as_u64()
            .expect("span start should be numeric") as usize;
        assert_ne!(
            start,
            comment_start + "// ".len(),
            "comment text must not be used as boundary span evidence:\n{}",
            boundary
        );
        assert!(
            start >= first_call,
            "boundary span should be anchored to an executable call site, not comments/strings:\n{}",
            boundary
        );
    }
}

#[test]
fn replay_critical_boundary_discovery_uses_full_cargo_dependency_breadth() {
    let project = TestProject::new("ecosystem-boundary-policy-target-dependency");
    project.write(
        "Kobo.toml",
        r#"[target."cfg(windows)".dependencies]
http_alias = { package = "reqwest", version = "0.12" }
"#,
    );
    let file = project.main_file(
        r#"
use http_alias::Client;

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
        "target-specific aliased dependencies should be replay-boundary candidates",
    );
    assert_contains(
        &output.combined(),
        "K0107",
        "target dependency aliases should produce replay boundary diagnostics",
    );
    assert_contains(
        &output.combined(),
        "http_alias",
        "check should discover target dependency aliases as external boundaries",
    );
}

#[test]
fn invalid_ecosystem_policy_emits_k0120() {
    let project = TestProject::new("ecosystem-invalid-ecosystem-policy");
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
        "invalid policy should use the ecosystem ecosystem parse diagnostic",
    );
}

#[test]
fn model_boundary_without_adapter_emits_k0123() {
    let project = TestProject::new("ecosystem-model-missing-adapter");
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
    let project = TestProject::new("ecosystem-typed-missing-declaration");
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
        "typed boundary without declaration must use the ecosystem missing-declaration diagnostic",
    );
}

#[test]
fn activity_boundary_records_result_but_not_external_internals() {
    let project = TestProject::new("ecosystem-activity-boundary");
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
    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"

[[activity]]
path = "sqlx::Client::new"
retry = "caller"
idempotency = "idempotent-connect"
result = "record"
compensation = "drop-client"
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
