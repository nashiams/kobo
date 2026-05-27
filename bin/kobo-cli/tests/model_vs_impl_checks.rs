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

fn run_dual_witness(project: &TestProject, source: &str, target: &str) -> (CliOutput, Value) {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1307"),
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

fn dual_source(model_lines: &str, scenario_body: &str) -> String {
    format!(
        r#"
ward DualWard {{
    state events: Vec<String>
{model_lines}

    scenario dual_case {{
{scenario_body}
    }}
}}
"#
    )
}

fn ward_delivery_source(model_lines: &str, scenario_body: &str) -> String {
    format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

ward DeliveryModel {{
{model_lines}

    scenario dual_obligation_case {{
{scenario_body}
    }}
}}
"#
    )
}

#[test]
fn dual_run_matches_equivalent_model_and_implementation() {
    let project = TestProject::new("model-dual-match");
    let source = dual_source("    model event deterministic-task", "        ward.task();");

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_success(&output, "matching model and implementation should pass");
    let comparison = &witness["model_vs_implementation"];
    assert_eq!(comparison["status"], "matched");
    assert_eq!(comparison["selection"]["source"], "ward_model");
    assert_eq!(comparison["model_run"]["source"], "ward_model");
    assert_eq!(comparison["trace"]["status"], "matched");
    assert_contains(
        &comparison["trace"]["implementation_events"].to_string(),
        "deterministic-task",
        "implementation side should come from the real scenario trace",
    );
}

#[test]
fn dual_run_executes_structured_ward_model_block() {
    let project = TestProject::new("model-dual-structured-model");
    let source = dual_source(
        "    model {\n        event deterministic-task;\n    }",
        "        ward.task();",
    );

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_success(&output, "structured model block should execute and match");
    assert_eq!(witness["model_vs_implementation"]["status"], "matched");
    assert_eq!(
        witness["model_vs_implementation"]["model_run"]["engine"],
        "ward-model-interpreter"
    );
    assert_eq!(
        witness["model_vs_implementation"]["model_run"]["steps_executed"],
        Value::from(1)
    );
}

#[test]
fn dual_run_interprets_model_transitions_not_expectation_lines() {
    let project = TestProject::new("model-dual-transition-model");
    let source = dual_source(
        "    model {\n        ward.task();\n    }",
        "        ward.task();",
    );

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_success(
        &output,
        "model transition steps should execute into comparable events",
    );
    let comparison = &witness["model_vs_implementation"];
    assert_eq!(comparison["status"], "matched");
    assert_contains(
        &comparison["model_run"]["events"].to_string(),
        "deterministic-task",
        "model interpreter should emit the transition event, not copy the source token",
    );
    assert_contains(
        &comparison["model_run"]["ir"].to_string(),
        "ward.task",
        "witness should expose the typed model IR step",
    );
}

#[test]
fn dual_run_records_typed_state_transitions_and_scheduler_assumptions() {
    let project = TestProject::new("model-dual-typed-model-ir");
    let source = dual_source(
        "    model {\n        state slice pending;\n        scheduler async;\n        transition dispatch -> ward.task;\n    }",
        "        ward.task();",
    );

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_success(
        &output,
        "typed ward model state and transition IR should execute and match",
    );
    let model_run = &witness["model_vs_implementation"]["model_run"];
    assert_eq!(model_run["semantics"], "typed_ward_model_ir");
    assert_contains(
        &model_run["ir"].to_string(),
        "transition",
        "model IR should preserve the named transition",
    );
    assert_contains(
        &model_run["states"].to_string(),
        "slice",
        "model IR should record typed state facts",
    );
    assert_eq!(model_run["scheduler_assumptions"]["preset"], "async");
    assert_eq!(
        model_run["scheduler_assumptions"]["same_as_implementation"],
        Value::Bool(true)
    );
}

#[test]
fn dual_run_reports_trace_divergence_with_source_spans() {
    let project = TestProject::new("model-dual-trace-divergence");
    let source = dual_source(
        "    model event expected-other-event",
        "        ward.task();",
    );

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_failure(&output, "trace divergence should fail the dual run");
    assert_eq!(witness["model_vs_implementation"]["status"], "diverged");
    assert_eq!(
        witness["model_vs_implementation"]["trace"]["first_difference"]["model"],
        "expected-other-event"
    );
    assert_eq!(
        witness["model_vs_implementation"]["trace"]["first_difference"]["implementation"],
        "deterministic-task"
    );
    assert_eq!(
        witness["model_vs_implementation"]["trace"]["source_span"]["mapped"],
        Value::Bool(true)
    );
}

#[test]
fn dual_run_reports_obligation_state_divergence() {
    let project = TestProject::new("model-dual-obligation-divergence");
    let source = ward_delivery_source(
        "    model obligation delivery leaked",
        "    let delivery = Delivery {};\n    delivery.ack();",
    );

    let (output, witness) = run_dual_witness(&project, &source, "dual_obligation_case");
    assert_failure(&output, "obligation state divergence should fail");
    assert_eq!(
        witness["model_vs_implementation"]["obligations"]["first_difference"]["binding"],
        "delivery"
    );
    assert_eq!(
        witness["model_vs_implementation"]["obligations"]["first_difference"]["model"],
        "leaked"
    );
    assert_eq!(
        witness["model_vs_implementation"]["obligations"]["first_difference"]["implementation"],
        "discharged"
    );
}

#[test]
fn dual_run_downgrades_when_boundary_policy_blocks_comparison() {
    let project = TestProject::new("model-dual-boundary-downgrade");
    let source = format!(
        "{}\n{}",
        r#"
#[kobo::boundary(crate = "outside_api", policy = "opaque", reason = "outside comparison scope")]
use outside_api::Client;
"#,
        dual_source(
            "    model event deterministic-task",
            "        ward.task();\n        Client::send();",
        )
    );

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_success(
        &output,
        "opaque boundary should downgrade comparison instead of pretending certainty",
    );
    assert_eq!(witness["model_vs_implementation"]["status"], "partial");
    assert_eq!(
        witness["model_vs_implementation"]["boundary_policy"]["status"],
        "downgraded"
    );
    assert_contains(
        &witness["model_vs_implementation"]["boundary_policy"]["decisions"].to_string(),
        "outside_api",
        "boundary policy evidence should be visible",
    );
}

#[test]
fn dual_run_uses_same_scheduler_seed_for_model_and_implementation() {
    let project = TestProject::new("model-dual-same-seed");
    let source = dual_source("    model event deterministic-task", "        ward.task();");

    let (output, witness) = run_dual_witness(&project, &source, "dual_case");
    assert_success(&output, "same-seed matching comparison should pass");
    assert_eq!(
        witness["model_vs_implementation"]["scheduler_seed"]["model"],
        Value::from(1307)
    );
    assert_eq!(
        witness["model_vs_implementation"]["scheduler_seed"]["implementation"],
        Value::from(1307)
    );
    assert_eq!(
        witness["model_vs_implementation"]["scheduler_seed"]["same_seed"],
        Value::Bool(true)
    );
}
