mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo, s,
    TestProject,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("service_runtime")
}

fn fixture(name: &str) -> PathBuf {
    fixture_root().join(name)
}

fn read_first_witness(project: &TestProject) -> Value {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness/report should exist");
    serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

fn write_first_witness(project: &TestProject, value: &Value) -> PathBuf {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness/report should exist");
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(value).expect("witness should serialize"),
    )
    .expect("witness should write");
    witness_path
}

#[test]
fn public_v10_fixture_acceptance_flows_are_real_files() {
    let project = TestProject::new("service-runtime-public-fixtures");
    for name in [
        "durable_queue.kobo",
        "async_gateway.kobo",
        "tiny_ward.kobo",
        "backend_adapter.kobo",
        "trap_select_boundary.kobo",
    ] {
        let path = fixture(name);
        assert!(
            path.exists(),
            "service runtime fixture `{}` must exist",
            path.display()
        );
    }

    let durable_queue = fixture("durable_queue.kobo");
    let durable_quick = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("7"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&durable_queue),
        ],
        &project.root,
    );
    assert_failure(
        &durable_quick,
        "durable queue fixture should expose crash/lost-delivery witness",
    );
    assert_contains(
        &durable_quick.combined(),
        "lost-message",
        "durable queue fixture must fail through storage state semantics",
    );

    let durable_deep = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("7"),
            path_arg(&durable_queue),
        ],
        &project.root,
    );
    assert_failure(
        &durable_deep,
        "deep durable queue fixture should also expose the modeled storage bug",
    );

    let async_gateway = fixture("async_gateway.kobo");
    let async_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("11"),
            path_arg(&async_gateway),
        ],
        &project.root,
    );
    assert_failure(
        &async_output,
        "async gateway fixture should expose orphan/unresolved reply witness",
    );
    assert_contains(
        &async_output.combined(),
        "unresolved-reply",
        "async gateway failure mode should be user-visible",
    );

    let tiny_ward = fixture("tiny_ward.kobo");
    let tiny_output = run_kobo(
        &[s("test"), s("--sim"), s("exhaustive"), path_arg(&tiny_ward)],
        &project.root,
    );
    assert_failure(
        &tiny_output,
        "tiny exhaustive fixture should find the failing bounded schedule",
    );
    assert_contains(
        &tiny_output.combined(),
        "scheduler",
        "tiny fixture failure should come from scheduler exploration",
    );

    let inspect = run_kobo(
        &[s("inspect"), s("--sim"), path_arg(&async_gateway)],
        &project.root,
    );
    assert_success(&inspect, "fixture inspect --sim should expose facades");
    assert_contains(
        &inspect.combined(),
        "simulation facade",
        "inspect output must show generated facade surface",
    );
}

#[test]
fn deep_scheduler_finds_interleaving_that_quick_does_not() {
    let project = TestProject::new("v10-schedule-sensitive");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn schedule_sensitive_delivery() {
    let delivery = Delivery {};
    ward.task();
    delivery.ack();
}
"#,
    );

    let quick = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("19"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &quick,
        "quick should run the cheap schedule where the obligation is discharged",
    );

    let deep = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("19"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &deep,
        "deep should explore an interleaving where the task boundary leaks the obligation",
    );
    assert_contains(
        &deep.combined(),
        "scheduler",
        "deep failure must be schedule-derived, not a static liveness label",
    );
}

#[test]
fn storage_commit_order_controls_durable_loss() {
    let project = TestProject::new("v10-storage-state-machine");
    let failing = project.write(
        "src/failing.kobo",
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn ack_before_commit_loses_delivery() {
    let delivery = Delivery {};
    delivery.ack();
    ward.storage.write("pending");
    ward.storage.crash_after_write();
}
"#,
    );
    let passing = project.write(
        "src/passing.kobo",
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn commit_before_crash_recovers_delivery() {
    let delivery = Delivery {};
    delivery.ack();
    ward.storage.write("pending");
    ward.storage.commit("pending");
    ward.storage.crash_after_write();
    ward.storage.recover();
}
"#,
    );

    let lost = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("5"),
            path_arg(&failing),
        ],
        &project.root,
    );
    assert_failure(&lost, "ack-before-commit crash must fail");
    assert_contains(
        &lost.combined(),
        "ack before durable commit",
        "failure should prove storage ordering, not only see a crash method name",
    );

    let recovered = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("5"),
            path_arg(&passing),
        ],
        &project.root,
    );
    assert_success(
        &recovered,
        "commit-before-crash recovery should be safe under the storage model",
    );
}

#[test]
fn network_delivery_state_changes_with_drop_delay_reorder() {
    let project = TestProject::new("v10-network-state-machine");
    let delivered = project.write(
        "src/delivered.kobo",
        r#"
#[kobo::scenario(profile = "network")]
fn delivered_reply() {
    ward.network.send("reply");
    ward.network.delay("reply");
    ward.network.reorder("reply");
    ward.network.receive("reply");
}
"#,
    );
    let dropped = project.write(
        "src/dropped.kobo",
        r#"
#[kobo::scenario(profile = "network")]
fn dropped_reply() {
    ward.network.send("reply");
    ward.network.drop("reply");
    ward.network.receive("reply");
}
"#,
    );

    let delivered_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("3"),
            s("--events=json"),
            path_arg(&delivered),
        ],
        &project.root,
    );
    assert_success(&delivered_output, "delayed/reordered reply should deliver");
    let delivered_text = delivered_output.combined();
    assert_contains(
        &delivered_text,
        "network-queued",
        "network send should enqueue message state",
    );
    assert_contains(
        &delivered_text,
        "network-delivered",
        "network receive should deliver queued message state",
    );

    let dropped_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("3"),
            path_arg(&dropped),
        ],
        &project.root,
    );
    assert_failure(&dropped_output, "receiving a dropped reply should fail");
    assert_contains(
        &dropped_output.combined(),
        "network delivery failed",
        "network failure must depend on message state",
    );
}

#[test]
fn public_backend_commands_report_stable_execution_truth() {
    let project = TestProject::new("v10-backend-truth");
    let output = run_kobo(&[s("sim"), s("backends"), s("--json")], &project.root);
    assert_success(&output, "sim backends --json should succeed");
    let value: Value = serde_json::from_str(&output.stdout).expect("backend JSON should parse");

    assert!(
        value.get("version").is_none(),
        "public backend output should not expose roadmap-stage version fields: {value}"
    );
    assert_eq!(value["executed"], true);
    assert!(
        value["backends"]
            .as_array()
            .is_some_and(|backends| backends
                .iter()
                .any(|backend| backend["name"] == "generated-rust-process"
                    && backend["executes_now"] == true
                    && backend["execution_status"] == "executable")),
        "backend list should expose the actual executed harness path: {value}"
    );
    assert!(
        value["backends"].as_array().is_some_and(|backends| backends
            .iter()
            .any(|backend| backend["name"] == "loom"
                && backend["executes_now"] == true
                && backend["execution_status"] == "executable"
                && backend["integration_level"] == "generated-user-rust"
                && backend["scenario_execution"] == "generated-user-rust-loom")),
        "phase 09 Loom support must execute generated user Rust inside Loom, not a lowered scenario model: {value}"
    );
    assert!(
        !output.combined().contains("v0.9"),
        "public backend output must not report stale v0.9-only execution status"
    );
}

#[test]
fn loom_backend_adapter_is_a_real_workspace_dependency() {
    let lock_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("Cargo.lock");
    let lock = fs::read_to_string(lock_path).expect("Cargo.lock should be readable");
    assert_contains(
        &lock,
        "name = \"loom\"",
        "v0.10 phase 09 must use a real Loom dependency, not only a backend label",
    );
}

#[test]
fn loom_backend_digest_discloses_generated_scenario_execution() {
    let project = TestProject::new("v10-loom-boundary");
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("23"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&fixture("backend_adapter.kobo")),
        ],
        &project.root,
    );
    assert_failure(&output, "sync backend fixture should emit a witness");

    let witness = read_first_witness(&project);
    assert_eq!(witness["backend_profile"], "sync");
    assert_eq!(witness["backend"], "loom");
    assert_eq!(
        witness["execution_digest"]["harness_engine"], "generated-rust-loom-process",
        "v0.10 sync backend must bind generated user Rust into Loom execution"
    );
}

#[test]
fn sim_core_library_uses_typed_error_not_anyhow() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let sim_core = repo_root.join("crates/compiler/kobo-sim-core");
    let cargo_toml =
        fs::read_to_string(sim_core.join("Cargo.toml")).expect("sim-core manifest should read");
    assert_not_contains(
        &cargo_toml,
        "anyhow",
        "library crate must not depend on anyhow",
    );
    assert_contains(
        &cargo_toml,
        "thiserror",
        "library crate should expose a typed thiserror error",
    );

    for source in ["src/core.rs", "src/harness.rs", "src/backend.rs"] {
        let text = fs::read_to_string(sim_core.join(source)).expect("sim-core source should read");
        assert_not_contains(
            &text,
            "anyhow::",
            "library crate APIs must not return anyhow errors",
        );
    }
}

#[test]
fn exact_witness_events_have_stable_ids() {
    let project = TestProject::new("v10-event-ids");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn async_gateway() {
    let reply = ReplyToken {};
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("11"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "async gateway should emit witness");
    let witness = read_first_witness(&project);
    let events = witness["event_stream"]
        .as_array()
        .expect("witness event stream should be an array");
    assert!(!events.is_empty(), "witness should contain replay events");
    for (expected_id, event) in events.iter().enumerate() {
        assert_eq!(
            event["id"].as_u64(),
            Some(expected_id as u64),
            "event IDs should be stable and sequential in {witness}"
        );
    }
}

#[test]
fn replay_rejects_shrink_that_removes_semantic_event() {
    let project = TestProject::new("v10-shrink-necessary-event");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn async_gateway() {
    let reply = ReplyToken {};
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("11"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "async gateway should emit witness");

    let mut witness = read_first_witness(&project);
    let shrunk_count = witness["event_stream"]
        .as_array()
        .map(Vec::len)
        .expect("event stream should be present") as u64;
    witness["shrink"]["removed_event_ids"] = serde_json::json!([0]);
    witness["shrink"]["original_event_count"] = serde_json::json!(shrunk_count + 1);
    witness["shrink"]["shrunk_event_count"] = serde_json::json!(shrunk_count);
    witness["shrink"]["replay_checked"] = serde_json::json!(true);
    let witness_path = write_first_witness(&project, &witness);

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
        "replay must reject shrink metadata that removes semantic evidence",
    );
    assert_contains(
        &replay.combined(),
        "K0106",
        "unsafe semantic-event removal should be a shrink diagnostic, not replay divergence",
    );
}

#[test]
fn witness_records_function_summaries_and_operation_coverage() {
    let project = TestProject::new("v10-summary-coverage");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

fn helper(delivery: Delivery) {
    delivery.ack();
}

#[kobo::scenario(profile = "async")]
fn routed_delivery() {
    let delivery = Delivery {};
    helper(delivery);
    let other = Delivery {};
    let _lost = other;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("17"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "unresolved second delivery should emit witness");
    let witness = read_first_witness(&project);

    assert!(
        witness["function_summaries"]
            .as_array()
            .is_some_and(|summaries| !summaries.is_empty()),
        "witness should carry reusable function summaries: {witness}"
    );
    assert!(
        witness["operation_coverage"]["modeled"]
            .as_array()
            .is_some_and(|modeled| modeled.iter().any(|entry| entry == "obligation-transfer")),
        "coverage should classify modeled operation kinds: {witness}"
    );
}

#[test]
fn deep_shrinker_removes_replay_irrelevant_network_ordering() {
    let project = TestProject::new("v10-shrink-network-ordering");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "network")]
fn network_gateway() {
    let reply = ReplyToken {};
    ward.network.send("reply");
    ward.network.delay("reply");
    ward.network.reorder("reply");
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("29"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "deep network gateway should emit witness");
    let witness = read_first_witness(&project);
    assert_contains(
        &witness["shrink"].to_string(),
        "independent-network-ordering",
        "deep shrink should minimize replay-irrelevant network delay/reorder events",
    );
}

#[test]
fn select_boundary_refuses_exact_until_modeled() {
    let project = TestProject::new("v10-select-boundary");
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("13"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&fixture("trap_select_boundary.kobo")),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "tokio::select-shaped fixture should not claim exact replay while select is unsupported",
    );
    let text = output.combined();
    assert_contains(&text, "K0116", "select coverage gap should be diagnostic");
    assert_contains(
        &text,
        "select",
        "diagnostic should name the unsupported select boundary",
    );
    let witness = read_first_witness(&project);
    let events = witness["events"].to_string();
    for expected in [
        "scheduler-select-branch",
        "scheduler-select-cancelled-branch",
        "scheduler-future-dropped",
    ] {
        assert_contains(
            &events,
            expected,
            "select-shaped source should still feed scheduler branch/drop state before exactness is refused",
        );
    }
}

#[test]
fn select_coverage_is_compiler_owned() {
    let text_only_project = TestProject::new("v10-select-text-only");
    let text_only = text_only_project.main_file(
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {}

#[kobo::scenario(profile = "async")]
fn text_only_select_reference() {
    // tokio::select! in a comment must not affect replay coverage.
    let _doc = "tokio::select! in a string must not affect replay coverage";
    let tx = Transaction {};
    let _lost = tx;
}
"#,
    );
    let text_only_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("31"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&text_only),
        ],
        &text_only_project.root,
    );
    assert_failure(
        &text_only_output,
        "text-only select reference should fail for liveness, not coverage",
    );
    assert_not_contains(
        &text_only_output.combined(),
        "K0116",
        "comment/string select text must not produce unsupported select coverage",
    );
    let text_only_witness = read_first_witness(&text_only_project);
    assert_not_contains(
        &text_only_witness["coverage"]["unsupported_constructs"].to_string(),
        "tokio::select!",
        "witness coverage must come from parsed/lowered facts, not source text",
    );

    let parsed_project = TestProject::new("v10-select-parsed");
    let parsed_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("31"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&fixture("trap_select_boundary.kobo")),
        ],
        &parsed_project.root,
    );
    assert_failure(
        &parsed_output,
        "parsed tokio::select! should still downgrade exact replay",
    );
    assert_contains(
        &parsed_output.combined(),
        "K0116",
        "actual parsed select macro should produce unsupported select coverage",
    );
    let parsed_witness = read_first_witness(&parsed_project);
    assert_contains(
        &parsed_witness["coverage"]["unsupported_constructs"].to_string(),
        "tokio::select!",
        "parsed select coverage must remain compiler-owned and visible",
    );
}
