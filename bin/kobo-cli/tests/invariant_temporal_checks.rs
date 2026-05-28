mod cli_test_support;

use std::path::Path;
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn run_trace_witness(project: &TestProject, source: &str, target: &str) -> (CliOutput, Value) {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1306"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(target),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness =
        serde_json::from_str(&std::fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    (output, witness)
}

fn trace_source(check_line: &str) -> String {
    format!(
        r#"
ward TraceWard {{
    state events: Vec<String>
    {check_line}

    scenario trace_case {{
        ward.task();
    }}
}}
"#
    )
}

#[test]
fn ward_invariant_appears_in_witness_output_with_source_span() {
    let project = TestProject::new("model-invariant-visible");
    let source = trace_source("invariant task_event_visible { eventually deterministic-task }");

    let (output, witness) = run_trace_witness(&project, &source, "trace_case");
    assert_success(&output, "passing invariant should not fail the run");
    let invariant = &witness["invariant_checks"][0];
    assert_eq!(invariant["name"], "task_event_visible");
    assert_eq!(invariant["status"], "passed");
    assert_eq!(invariant["source"], "ward_model");
    assert_eq!(invariant["source_span"]["mapped"], Value::Bool(true));
    assert_contains(
        invariant["source_span"]["snippet"]
            .as_str()
            .unwrap_or_default(),
        "task_event_visible",
        "invariant source span should point to original check",
    );
}

#[test]
fn invariant_failure_reports_counterexample_trace() {
    let project = TestProject::new("model-invariant-failure");
    let source = trace_source("invariant no_task_event { never deterministic-task }");

    let (output, witness) = run_trace_witness(&project, &source, "trace_case");
    assert_failure(&output, "violated invariant should fail the run");
    assert_eq!(witness["failure"]["mode"], "invariant_failure");
    assert_eq!(witness["invariant_checks"][0]["status"], "failed");
    assert_contains(
        &witness["invariant_checks"][0]["trace_excerpt"].to_string(),
        "deterministic-task",
        "invariant failure should include a counterexample trace excerpt",
    );
}

#[test]
fn temporal_always_check_fails_on_violating_event() {
    let project = TestProject::new("model-temporal-always");
    let source = trace_source("temporal always deterministic-task");

    let (output, witness) = run_trace_witness(&project, &source, "trace_case");
    assert_failure(&output, "always check should fail on a different event");
    assert_eq!(witness["temporal_checks"][0]["kind"], "always");
    assert_eq!(witness["temporal_checks"][0]["status"], "failed");
}

#[test]
fn temporal_eventually_check_fails_when_event_never_occurs() {
    let project = TestProject::new("model-temporal-eventually");
    let source = trace_source("temporal eventually missing-event");

    let (output, witness) = run_trace_witness(&project, &source, "trace_case");
    assert_failure(&output, "eventually check should fail when event is absent");
    assert_eq!(witness["temporal_checks"][0]["kind"], "eventually");
    assert_eq!(witness["temporal_checks"][0]["status"], "failed");
    assert_contains(
        witness["temporal_checks"][0]["message"]
            .as_str()
            .unwrap_or_default(),
        "missing-event",
        "temporal failure should name the missing event",
    );
}

#[test]
fn temporal_never_check_fails_when_forbidden_event_occurs() {
    let project = TestProject::new("model-temporal-never");
    let source = trace_source("temporal never deterministic-task");

    let (output, witness) = run_trace_witness(&project, &source, "trace_case");
    assert_failure(&output, "never check should fail on forbidden event");
    assert_eq!(witness["temporal_checks"][0]["kind"], "never");
    assert_contains(
        &witness["temporal_checks"][0]["trace_excerpt"].to_string(),
        "deterministic-task",
        "never failure should include the forbidden event",
    );
}

#[test]
fn invariant_failure_is_distinct_from_replay_mismatch() {
    let project = TestProject::new("model-invariant-not-replay-mismatch");
    let source = trace_source("invariant no_task_event { never deterministic-task }");

    let (output, witness) = run_trace_witness(&project, &source, "trace_case");
    assert_failure(&output, "invariant failure should fail");
    assert_contains(
        &output.combined(),
        "K0108",
        "invariant failure should use K0108",
    );
    assert_eq!(witness["failure"]["mode"], "invariant_failure");
    assert!(
        !output.combined().contains("K0104") && !output.combined().contains("K0117"),
        "invariant failure must be distinct from replay mismatch diagnostics"
    );
}
