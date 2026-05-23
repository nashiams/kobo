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
fn ecosystem_backends_distinguish_native_execution_from_reserved_pins() {
    let project = TestProject::new("v10-backend-registry-executable");
    let output = run_kobo(&[s("sim"), s("backends"), s("--json")], &project.root);
    assert_success(&output, "backend registry should render");
    assert_not_contains(
        &output.stdout,
        "v0.",
        "backend registry should not expose roadmap-stage version wording",
    );
    assert_not_contains(
        &output.stdout,
        "executes_in_v10",
        "backend registry should use stable product schema names",
    );
    let value: Value = serde_json::from_str(&output.stdout).expect("backend JSON should parse");
    assert!(
        value.get("version").is_none(),
        "backend registry should not publish roadmap version field: {value}"
    );
    let backends = value["backends"]
        .as_array()
        .expect("backend list should be an array");

    let loom = backends
        .iter()
        .find(|backend| backend["name"] == "loom")
        .unwrap_or_else(|| panic!("backend `loom` should be listed: {value}"));
    assert_eq!(
        loom["executes_now"], true,
        "Loom is the native backend linked into the workspace: {loom}"
    );
    assert_eq!(loom["execution_status"], "executable");
    assert_eq!(loom["integration_level"], "generated-user-rust");

    for name in ["shuttle", "turmoil", "madsim"] {
        let backend = backends
            .iter()
            .find(|backend| backend["name"] == name)
            .unwrap_or_else(|| panic!("backend `{name}` should be listed: {value}"));
        assert_eq!(
            backend["executes_now"], false,
            "{name} must not claim native execution until the adapter is linked: {backend}"
        );
        assert_eq!(backend["execution_status"], "reserved");
        assert_eq!(backend["integration_level"], "metadata-only");
        assert_eq!(backend["scenario_execution"], "unsupported-native-adapter");
    }
}

#[test]
fn async_witness_records_execution_backend_separately_from_reserved_fit() {
    let project = TestProject::new("v10-async-execution-backend");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject)]
struct Response {}

#[kobo::scenario(profile = "async")]
async fn async_route() {
    let response = Response {};
    let _lost = response;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("5"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "async witness should be written for liveness failure",
    );

    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert_eq!(witness["backend_profile"], "async");
    assert_eq!(witness["backend"], "generated-rust-process");
    assert_eq!(
        witness["backend_controls"]["backend"],
        "generated-rust-process"
    );
    assert_eq!(
        witness["backend_version"]["backend"],
        "generated-rust-process"
    );
    assert_eq!(witness["reserved_backend_fit"][0]["backend"], "shuttle");
    assert_eq!(witness["reserved_backend_fit"][0]["status"], "reserved");
    assert_ne!(
        witness["backend"], "shuttle",
        "metadata-only Shuttle must not be recorded as the execution backend"
    );
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
            backend["ecosystem_scope"], "backend-native-unsupported",
            "{name} should report adapter scope explicitly: {backend}"
        );
    }
}

#[test]
fn expert_loom_backend_flags_drive_native_scheduler_output_without_source_imports() {
    let project = TestProject::new("v10-expert-backend-flags");
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
            s("exhaustive"),
            s("--backend"),
            s("loom"),
            s("--scheduler"),
            s("exhaustive"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "explicit Loom backend and scheduler should run");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["backend"], "loom");
    assert_eq!(json["backend_profile"], "sync");
    assert_eq!(json["scheduler"]["strategy"], "exhaustive");

    let source = project.read("src/main.kobo");
    assert_not_contains(&source, "shuttle::", "user source must not import Shuttle");
    assert_not_contains(&source, "loom::", "user source must not import Loom");
}

#[test]
fn default_sim_success_output_hides_backend_choices() {
    let project = TestProject::new("v10-default-output-kobo-first");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn default_output_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[s("test"), s("--sim"), s("quick"), path_arg(&file)],
        &project.root,
    );

    assert_success(&output, "default sim success output should succeed");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["status"], "passed");
    assert_eq!(json["sim_profile"], "quick");
    assert!(
        json.get("backend").is_none(),
        "default output should not show backend internals: {json}"
    );
    assert!(
        json.get("backend_profile").is_none(),
        "default output should not show backend profile internals: {json}"
    );
    assert!(
        json.get("scheduler").is_none(),
        "default output should not show scheduler internals: {json}"
    );
}

#[test]
fn show_backend_choices_opts_into_success_backend_details() {
    let project = TestProject::new("v10-show-backend-choices");
    project.write(
        "Kobo.toml",
        r#"[sim]
show_backend_choices = true
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn visible_backend_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[s("test"), s("--sim"), s("quick"), path_arg(&file)],
        &project.root,
    );

    assert_success(&output, "sim output should succeed");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["backend"], "loom");
    assert_eq!(json["backend_profile"], "sync");
    assert_eq!(json["scheduler"]["profile"], "quick");
}

#[test]
fn unsupported_native_backend_scheduler_reports_explicit_debt_path() {
    let project = TestProject::new("v10-unsupported-native-backend-scheduler");
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
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "unsupported Shuttle scheduler should not be accepted as metadata",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "unsupported backend option",
        "failure should name unsupported backend controls",
    );
    assert_contains(
        &text,
        "adapter is not linked",
        "failure should be honest about missing native adapter",
    );
    assert_contains(
        &text,
        "scenario debt",
        "failure should offer scenario debt as an explicit path",
    );
}

#[test]
fn unsupported_native_backend_pin_reports_explicit_debt_path_without_scheduler() {
    let project = TestProject::new("v10-unsupported-native-backend-pin");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn reserved_backend_route() {
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
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "unsupported native backend pin should fail without requiring a scheduler knob",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "unsupported backend option",
        "failure should name unsupported backend",
    );
    assert_contains(
        &text,
        "unsupported-native-adapter",
        "failure should disclose adapter status",
    );
    assert_contains(&text, "scenario debt", "failure should offer scenario debt");
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
fn inspect_harness_rejects_unsupported_backend_pin() {
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

    assert_failure(
        &output,
        "inspect should reject unsupported backend-native pins",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "unsupported backend option",
        "inspect failure should name unsupported backend controls",
    );
    assert_contains(
        &text,
        "scenario debt",
        "inspect failure should offer scenario debt as an explicit path",
    );
    assert_contains(
        &text,
        "unsupported-native-adapter",
        "inspect failure should disclose that the adapter is metadata-only",
    );
}

#[test]
fn inspect_harness_reports_generated_manifest_and_source_path() {
    let project = TestProject::new("v10-inspect-generated-harness");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn inspect_generated_harness_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[s("inspect"), s("--sim"), s("--harness"), path_arg(&file)],
        &project.root,
    );

    assert_success(
        &output,
        "inspect should generate and report harness artifacts",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "generated harness manifest",
        "inspect output should point at the generated harness manifest",
    );
    assert_contains(
        &text,
        "harness_rs_path",
        "inspect output should point at the generated harness source",
    );
    assert_contains(
        &text,
        "backend_replay_token_hash",
        "inspect output should expose the backend replay token mapping",
    );

    let harness_path = text
        .lines()
        .find_map(|line| line.strip_prefix("// kobo: harness_rs_path: "))
        .expect("inspect output should contain a harness source path");
    let harness_source = fs::read_to_string(harness_path).expect("harness source should exist");
    assert_contains(
        &harness_source,
        "fn inspect_generated_harness_route()",
        "inspect should report a real generated harness that contains the target",
    );
}

#[test]
fn inspect_harness_backend_pin_drives_generated_profile() {
    let project = TestProject::new("v10-inspect-backend-pin-drives-profile");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn inspect_pinned_loom_route() {
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
            s("loom"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "supported backend pin should inspect generated harness",
    );
    let text = output.combined();
    let harness_path = text
        .lines()
        .find_map(|line| line.strip_prefix("// kobo: harness_rs_path: "))
        .expect("inspect output should contain a harness source path");
    let harness_source = fs::read_to_string(harness_path).expect("harness source should exist");
    assert_contains(
        &harness_source,
        "loom::model",
        "supported Loom backend pin should drive the generated harness profile",
    );
}

#[test]
fn sim_config_profiles_and_backend_knobs_drive_test_output() {
    let project = TestProject::new("v10-sim-config-schema");
    project.write(
        "Kobo.toml",
        r#"[sim]
default_profile = "quick"
show_backend_choices = true

[sim.profile.quick]
schedule_budget = 7
shrink = "off"

[sim.backend.loom]
enabled = true
scheduler = "exhaustive"
replay_token = "record"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn configured_sim_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "sim config should drive test output");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["backend"], "loom");
    assert_eq!(json["scheduler"]["strategy"], "exhaustive");
    assert_eq!(
        json["scheduler"]["event_budget"], 7,
        "sim.profile.quick.schedule_budget should set the scheduler budget"
    );
    assert_eq!(
        json["sim_config"]["backends"]["loom"]["replay_token"],
        "record"
    );
}

#[test]
fn sim_config_seed_count_runs_scheduler_portfolio() {
    let project = TestProject::new("v10-sim-config-seed-count");
    project.write(
        "Kobo.toml",
        r#"[sim]
default_profile = "quick"

[sim.profile.quick]
seed_count = 3
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn configured_seed_portfolio_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "seed_count should execute a seed portfolio");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["scheduler"]["seed_count"], 3);
    assert_eq!(
        json["sim_config"]["profiles"]["quick"]["seed_count"], 3,
        "stable sim config should be serialized for audit"
    );
    let seed_cases = json["events"]
        .as_array()
        .expect("events should be an array")
        .iter()
        .filter(|event| event["kind"] == "scheduler-seed-case")
        .map(|event| {
            event["value"]
                .as_u64()
                .expect("seed case should carry seed")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        seed_cases,
        vec![0, 1, 2],
        "sim.profile.quick.seed_count must drive executed scheduler seeds"
    );
}

#[test]
fn loom_scheduler_choice_changes_executed_trace() {
    let project = TestProject::new("v10-loom-scheduler-policy");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn scheduled_sync_route() {
    ward.task();
}
"#,
    );

    let small_random = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--profile"),
            s("sync"),
            s("--backend"),
            s("loom"),
            s("--scheduler"),
            s("small-random"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let exhaustive = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--profile"),
            s("sync"),
            s("--backend"),
            s("loom"),
            s("--scheduler"),
            s("exhaustive"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&small_random, "small-random scheduler run should pass");
    assert_success(&exhaustive, "exhaustive scheduler run should pass");
    let small_json: Value =
        serde_json::from_str(&small_random.stdout).expect("small-random JSON should parse");
    let exhaustive_json: Value =
        serde_json::from_str(&exhaustive.stdout).expect("exhaustive JSON should parse");
    let small_events = small_json["events"].to_string();
    let exhaustive_events = exhaustive_json["events"].to_string();
    assert_contains(
        &small_events,
        "scheduler-small-random-seed",
        "small-random scheduler must affect executed trace events",
    );
    assert_contains(
        &exhaustive_events,
        "scheduler-exhaustive-cap",
        "exhaustive scheduler must affect executed trace events",
    );
    assert_ne!(
        small_json["events"], exhaustive_json["events"],
        "scheduler policy should change the executed trace"
    );
}

#[test]
fn sim_backend_loom_max_branches_drives_exhaustive_budget() {
    let project = TestProject::new("v10-sim-config-max-branches");
    project.write(
        "Kobo.toml",
        r#"[sim]
default_profile = "exhaustive"

[sim.backend.loom]
enabled = true
scheduler = "exhaustive"
max_branches = 5
checkpoint_replay = true
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn configured_max_branches_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("exhaustive"),
            s("--events=json"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "configured Loom max_branches should execute without a CLI knob",
    );
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["scheduler"]["event_budget"], 5);
    assert_eq!(json["max_branches"], 5);
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    let harness_path = witness["harness_manifest"]["harness_rs_path"]
        .as_str()
        .expect("witness should include generated harness source path");
    let harness_source = fs::read_to_string(harness_path).expect("harness source should read");
    assert_contains(
        &harness_source,
        "loom::model::Builder::new",
        "Loom max_branches should use native Loom builder controls",
    );
    assert_contains(
        &harness_source,
        "max_branches = 5",
        "configured max_branches should be assigned to the Loom builder",
    );
}

#[test]
fn sim_backend_replay_token_none_suppresses_recorded_token_and_keeps_backend_version() {
    let project = TestProject::new("v10-sim-config-replay-token-none");
    project.write(
        "Kobo.toml",
        r#"[sim.backend.loom]
enabled = true
replay_token = "none"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn configured_replay_token_route() {
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

    assert_success(&output, "passing sync route should still emit a witness");
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert_eq!(witness["backend_controls"]["replay_token"], "none");
    assert_eq!(witness["backend_replay_token"], "none");
    assert_eq!(witness["backend_replay"], "none");
    assert_eq!(witness["backend_version"]["backend"], "loom");
    assert_eq!(
        witness["backend_version"]["adapter_source"],
        "kobo-sim-core"
    );
    assert!(
        witness["backend_version"]["adapter_version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "backend version metadata should include the linked adapter version: {witness}"
    );
}

#[test]
fn sim_backend_checkpoint_replay_is_validated_by_replay() {
    let project = TestProject::new("v10-sim-config-checkpoint-replay");
    project.write(
        "Kobo.toml",
        r#"[sim.backend.loom]
enabled = true
checkpoint_replay = true
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "sync")]
fn configured_checkpoint_route() {
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

    assert_success(&output, "checkpoint replay witness should emit");
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let mut witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert_eq!(witness["checkpoint_replay"]["enabled"], true);
    assert_eq!(
        witness["checkpoint_replay"]["semantic_trace_hash"],
        witness["execution_digest"]["semantic_trace_hash"],
        "checkpoint replay should bind the semantic trace digest"
    );
    witness["checkpoint_replay"]["semantic_trace_hash"] = Value::String("forged".to_owned());
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(&witness).unwrap(),
    )
    .expect("mutated witness should write");

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
        "replay must reject forged checkpoint replay metadata",
    );
    assert_contains(
        &replay.combined(),
        "checkpoint_replay",
        "checkpoint replay validation should name the forged metadata",
    );
}

#[test]
fn sim_config_default_profile_runs_without_cli_sim_flag() {
    let project = TestProject::new("v10-sim-default-profile");
    project.write(
        "Kobo.toml",
        r#"[sim]
default_profile = "deep"

[sim.profile.deep]
schedule_budget = 13
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn default_profile_route() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[s("test"), s("--events=json"), path_arg(&file)],
        &project.root,
    );

    assert_success(&output, "sim default profile should drive test");
    let json = serde_json::from_str::<Value>(&output.stdout).expect("test JSON should parse");
    assert_eq!(json["sim_profile"], "deep");
    assert_eq!(
        json["scheduler"]["event_budget"], 13,
        "default profile should use configured schedule budget"
    );
}
