mod cli_test_support;

use std::fs;

use cli_test_support::{
    assert_failure, assert_json_has_path, assert_success, path_arg, run_kobo, s, TestProject,
};
use serde_json::Value;

#[test]
fn obligation_events_record_create_transfer_discharge_and_coverage_buckets() {
    let project = TestProject::new("runtime-obligation-summary");
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
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "unresolved second delivery should emit witness");
    let witness = read_first_witness(&project);

    assert_json_has_path(
        &witness,
        &["obligation_events"],
        "obligation events should be serialized",
    );
    assert_json_has_path(
        &witness,
        &["scenario_coverage", "covered"],
        "coverage buckets should be serialized",
    );
    let text = serde_json::to_string(&witness).unwrap();
    for state in ["created", "transferred", "discharged", "leaked"] {
        assert!(
            text.contains(state),
            "obligation summary should include `{state}` in {text}"
        );
    }
}

#[test]
fn helper_discharge_is_a_semantic_function_summary_not_flat_event_projection() {
    let project = TestProject::new("runtime-helper-summary");
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
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("19"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "helper discharge should satisfy the obligation");
    let witness = read_first_witness(&project);
    let summaries = witness["function_summaries"]
        .as_array()
        .expect("function summaries should be an array");
    let helper_summary = summaries
        .iter()
        .find(|summary| summary["function"] == "helper")
        .unwrap_or_else(|| panic!("helper summary missing from {summaries:?}"));
    assert_eq!(
        helper_summary["transfers_in"][0], "delivery",
        "helper summary should record incoming transfer"
    );
    assert_eq!(
        helper_summary["discharges"][0], "delivery",
        "helper summary should own the helper discharge"
    );
    let target_summary = summaries
        .iter()
        .find(|summary| summary["function"] == "routed_delivery")
        .expect("target summary should exist");
    assert_eq!(
        target_summary["transfers"][0], "delivery->helper",
        "target summary should record the transfer edge"
    );
}

#[test]
fn scenario_coverage_changes_when_obligation_is_not_exercised() {
    let covered = TestProject::new("runtime-coverage-covered");
    let covered_file = covered.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn covered_delivery() {
    let delivery = Delivery {};
    delivery.ack();
}
"#,
    );
    let covered_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&covered_file),
        ],
        &covered.root,
    );
    assert_success(
        &covered_output,
        "covered obligation scenario should pass and still write report",
    );
    let covered_report = read_first_witness(&covered);

    let uncovered = TestProject::new("runtime-coverage-uncovered");
    let uncovered_file = uncovered.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn uncovered_delivery() {
    let delivery = Delivery {};
    let _lost = delivery;
}
"#,
    );
    let uncovered_output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&uncovered_file),
        ],
        &uncovered.root,
    );
    assert_failure(
        &uncovered_output,
        "uncovered obligation scenario should emit failure",
    );
    let uncovered_report = read_first_witness(&uncovered);

    assert_eq!(
        covered_report["scenario_coverage"]["covered"][0],
        "delivery"
    );
    assert!(
        covered_report["scenario_coverage"]["uncovered"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "covered scenario should not list uncovered obligations: {covered_report}"
    );
    assert_eq!(
        uncovered_report["scenario_coverage"]["uncovered"][0],
        "delivery"
    );
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
