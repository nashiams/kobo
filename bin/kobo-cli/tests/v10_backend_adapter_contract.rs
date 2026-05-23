mod v09_common;

use std::fs;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo, s,
    TestProject,
};

#[test]
fn sync_backend_adapter_writes_replay_token_and_stays_out_of_user_source() {
    let project = TestProject::new("v10-backend-adapter");
    let file = project.main_file(
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {}

#[kobo::scenario(profile = "sync")]
fn sync_transaction() {
    let tx = Transaction {};
    let _lost = tx;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("23"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "sync backend failure should emit witness");

    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert_eq!(witness["backend_profile"], "sync");
    assert_eq!(witness["backend"], "loom");
    assert_eq!(
        witness["execution_digest"]["harness_engine"],
        "generated-rust-loom-process",
        "sync profile should execute generated user Rust inside Loom, not only a lowered scenario model"
    );
    assert!(witness["backend_replay_token"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
    let harness_path = witness["harness_manifest"]["harness_rs_path"]
        .as_str()
        .expect("witness should include generated harness source path");
    let harness_source = fs::read_to_string(harness_path).expect("harness source should read");
    assert_contains(
        &harness_source,
        "loom::model(||",
        "generated sync harness must wrap the user target in Loom",
    );
    assert_contains(
        &harness_source,
        "fn sync_transaction()",
        "Loom harness must include generated user Rust, not only a lowered scenario state machine",
    );
    assert_not_contains(
        &harness_source,
        concat!("lowered-scenario", "-loom"),
        "Loom harness source must not be the old lowered scenario adapter",
    );

    let inspect = run_kobo(
        &[s("inspect"), s("--sim"), s("--harness"), path_arg(&file)],
        &project.root,
    );
    assert_success(
        &inspect,
        "inspect --sim --harness should show adapter boundary",
    );
    assert_contains(
        &inspect.combined(),
        "backend adapter",
        "inspect output should expose backend adapter boundary",
    );

    let source = project.read("src/main.kobo");
    assert_not_contains(&source, "loom::", "user source must not import Loom");
    assert_not_contains(&source, "shuttle::", "user source must not import Shuttle");
}

#[test]
fn ecosystem_backends_are_executable_v10_adapters_not_metadata_only() {
    let project = TestProject::new("v10-backend-registry-executable");
    let output = run_kobo(&[s("sim"), s("backends"), s("--json")], &project.root);
    assert_success(&output, "backend registry should render");
    let value: Value = serde_json::from_str(&output.stdout).expect("backend JSON should parse");
    let backends = value["backends"]
        .as_array()
        .expect("backend list should be an array");

    for name in ["loom", "shuttle", "turmoil", "madsim"] {
        let backend = backends
            .iter()
            .find(|backend| backend["name"] == name)
            .unwrap_or_else(|| panic!("backend `{name}` should be listed: {value}"));
        assert_eq!(
            backend["executes_in_v10"], true,
            "{name} should no longer be metadata-only: {backend}"
        );
        assert_ne!(
            backend["integration_level"], "metadata-only",
            "{name} should report an executable adapter level: {backend}"
        );
        assert_ne!(
            backend["scenario_execution"],
            concat!("not", "-linked"),
            "{name} should report the generated user Rust execution surface: {backend}"
        );
    }
}

#[test]
fn ecosystem_backend_registry_does_not_claim_full_external_crate_exploration() {
    let project = TestProject::new("v10-backend-registry-scope");
    let output = run_kobo(&[s("sim"), s("backends"), s("--json")], &project.root);
    assert_success(&output, "backend registry should render");
    let value: Value = serde_json::from_str(&output.stdout).expect("backend JSON should parse");
    let backends = value["backends"]
        .as_array()
        .expect("backend list should be an array");

    for name in ["shuttle", "turmoil", "madsim"] {
        let backend = backends
            .iter()
            .find(|backend| backend["name"] == name)
            .unwrap_or_else(|| panic!("backend `{name}` should be listed: {value}"));
        assert_eq!(
            backend["full_ecosystem_exploration"], false,
            "{name} adapter evidence must not claim arbitrary external crate exploration: {backend}"
        );
        assert_eq!(
            backend["ecosystem_scope"], "generated-user-rust-adapter",
            "{name} should report adapter scope explicitly: {backend}"
        );
    }
}

#[test]
fn expert_backend_flags_drive_scheduler_output_without_source_imports() {
    let project = TestProject::new("v10-expert-backend-flags");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn expert_async_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--backend"),
            s("shuttle"),
            s("--scheduler"),
            s("pct"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "explicit backend and scheduler should run");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["backend"], "shuttle");
    assert_eq!(json["backend_profile"], "async");
    assert_eq!(json["scheduler"]["strategy"], "pct");

    let source = project.read("src/main.kobo");
    assert_not_contains(&source, "shuttle::", "user source must not import Shuttle");
    assert_not_contains(&source, "loom::", "user source must not import Loom");
}

#[test]
fn unsupported_backend_scheduler_combo_reports_explicit_debt_path() {
    let project = TestProject::new("v10-unsupported-backend-knob");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn expert_sync_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--backend"),
            s("loom"),
            s("--scheduler"),
            s("pct"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "unsupported backend scheduler combo should fail");
    let text = output.combined();
    assert_contains(
        &text,
        "unsupported backend option",
        "failure should name unsupported backend controls",
    );
    assert_contains(
        &text,
        "scenario debt",
        "failure should offer scenario debt as an explicit path",
    );
    assert_contains(
        &text,
        "stable Kobo profile",
        "failure should offer the stable profile fallback",
    );
}

#[test]
fn inspect_harness_accepts_backend_pin_as_expert_transparency() {
    let project = TestProject::new("v10-inspect-backend-pin");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn inspect_backend_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("inspect"),
            s("--sim"),
            s("--harness"),
            s("--backend"),
            s("shuttle"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "inspect should accept a backend pin");
    let text = output.combined();
    assert_contains(
        &text,
        "backend adapter boundary",
        "inspect output should still describe the generated harness boundary",
    );
    assert_contains(
        &text,
        "backend pin: shuttle",
        "inspect output should disclose the expert backend pin",
    );
}
