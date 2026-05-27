mod cli_common;

use std::path::Path;
use std::time::Duration;

use cli_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo_with_timeout, s,
    CliOutput, TestProject,
};
use serde_json::Value;

const V13_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn run_witness(project: &TestProject, source: &str, target: &str) -> Value {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1305"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(target),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "ward fixture should run");
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    serde_json::from_str(&std::fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

fn ward_source() -> &'static str {
    r#"
ward DurableQueue {
    state log: Vec<String>
    state pending: Vec<String>

    obligation Delivery must ack | nack | requeue

    invariant no_lost_acked_messages { never lost_message }
    port storage: durable_log
    recording ack_log
    debt external_metrics

    scenario crash_during_ack {
        let delivery = Delivery {};
        delivery.ack();
    }
}
"#
}

fn attribute_source() -> &'static str {
    r#"
#[kobo::ward]
struct DurableQueue {
    log: Vec<String>,
    pending: Vec<String>,
}

#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "sync")]
fn crash_during_ack() {
    let delivery = Delivery {};
    delivery.ack();
}
"#
}

#[test]
fn ward_syntax_desugars_to_attribute_model() {
    let ward_project = TestProject::new("model-ward-desugar");
    let attr_project = TestProject::new("model-ward-attr-equivalent");

    let ward_witness = run_witness(&ward_project, ward_source(), "crash_during_ack");
    let attr_witness = run_witness(&attr_project, attribute_source(), "crash_during_ack");

    assert_eq!(
        normalized_obligations(&ward_witness),
        normalized_obligations(&attr_witness),
        "ward syntax should produce the same lifecycle model as attributes"
    );
    assert_contains(
        ward_witness["inferred_obligations"][0]["source_span"]["snippet"]
            .as_str()
            .unwrap_or_default(),
        "delivery",
        "ward syntax source coverage should resolve to original scenario text",
    );
    assert_eq!(ward_witness["strict_liveness"]["status"], "passed");
}

fn normalized_obligations(witness: &Value) -> Vec<Value> {
    witness["inferred_obligations"]
        .as_array()
        .expect("inferred obligations should exist")
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation["binding"].clone(),
                "kind": obligation["kind"].clone(),
                "state": obligation["state"].clone(),
                "template_id": obligation["template_id"].clone(),
                "template_schema": obligation["template_schema"].clone(),
                "schema_version": obligation["schema_version"].clone(),
                "terminal_actions": obligation["terminal_actions"].clone(),
            })
        })
        .collect()
}

#[test]
fn attribute_ward_form_remains_supported() {
    let project = TestProject::new("model-attribute-ward-supported");
    let witness = run_witness(&project, attribute_source(), "crash_during_ack");
    assert_eq!(witness["strict_liveness"]["status"], "passed");
    assert!(
        witness["formal_core"]["functions"]
            .as_array()
            .expect("formal core functions should exist")
            .iter()
            .any(|function| function["name"].as_str() == Some("crash_during_ack")),
        "attribute form should still lower to Core: {}",
        witness["formal_core"]
    );
}

#[test]
fn ward_syntax_preserves_scenario_profile_metadata() {
    let source = r#"
ward AsyncGateway {
    obligation ReplyToken must reply | reject | cancel

    scenario handle_request profile async {
        let token = ReplyToken {};
        token.reply();
    }
}
"#;
    let project = TestProject::new("model-ward-async-profile");
    let witness = run_witness(&project, source, "handle_request");
    assert_eq!(
        witness["scenario"]["profile"],
        Value::from("async"),
        "ward scenario profile metadata should select the async backend"
    );
    assert_eq!(
        witness["backend_profile"],
        Value::from("async"),
        "ward scenario profile should flow into backend selection"
    );

    let file = project.main_file(source);
    let output = run_kobo(
        &[s("inspect"), s("--scenario-metadata"), path_arg(&file)],
        &project.root,
    );
    assert_success(&output, "ward scenario metadata should inspect");
    assert_contains(
        &output.combined(),
        "scenario handle_request profile async",
        "inspect metadata should preserve the parsed scenario profile",
    );
}

#[test]
fn multi_module_ward_preserves_ports_recordings_and_debt() {
    let project = TestProject::new("model-multi-module-ward");
    let main = project.main_file(
        r#"
mod queue;

ward QueueScenarios {
    scenario crash_during_ack {
        let delivery = Delivery {};
        delivery.ack();
    }
}
"#,
    );
    project.write(
        "src/queue.kobo",
        r#"
ward DurableQueue {
    state log: Vec<String>
    state pending: Vec<String>

    obligation Delivery must ack | nack | requeue
    port storage: durable_log
    recording ack_log
    debt external_metrics
}
"#,
    );
    let output = run_kobo(
        &[s("inspect"), s("--scenario-metadata"), path_arg(&main)],
        &project.root,
    );
    assert_success(&output, "multi-module ward entry file should inspect");
    let text = output.combined();
    for expected in [
        "module src/queue.kobo",
        "ward DurableQueue",
        "ward QueueScenarios",
        "scenario crash_during_ack",
        "port storage",
        "recording ack_log",
        "debt external_metrics",
    ] {
        assert_contains(
            &text,
            expected,
            "ward metadata should preserve external facts",
        );
    }
}

#[test]
fn ward_inspect_reports_state_obligation_invariant_scenario_and_port_facts() {
    let project = TestProject::new("model-ward-inspect-facts");
    let file = project.main_file(ward_source());
    let output = run_kobo(
        &[s("inspect"), s("--scenario-metadata"), path_arg(&file)],
        &project.root,
    );
    assert_success(&output, "ward syntax should inspect");
    let text = output.combined();
    for expected in [
        "ward DurableQueue",
        "state log",
        "state pending",
        "obligation Delivery must ack | nack | requeue",
        "invariant no_lost_acked_messages",
        "scenario crash_during_ack",
        "port storage",
    ] {
        assert_contains(&text, expected, "inspect should report ward fact");
    }
}

#[test]
fn ward_clean_rust_output_preserves_zero_kobo_dependency_exit_ramp() {
    let project = TestProject::new("model-ward-clean-rust");
    let file = project.main_file(ward_source());
    let output = run_kobo(
        &[s("inspect"), s("--clean"), path_arg(&file)],
        &project.root,
    );
    assert_success(&output, "ward syntax should produce clean Rust");
    assert_contains(
        &output.stdout,
        "struct DurableQueue",
        "clean Rust should retain the ward data type",
    );
    assert_contains(
        &output.stdout,
        "fn crash_during_ack",
        "clean Rust should retain the scenario function",
    );
    assert_not_contains(
        &output.stdout,
        "kobo::",
        "clean Rust exit ramp should not depend on Kobo attributes",
    );
}

#[test]
fn ward_keyword_inside_comments_and_strings_is_not_desugared() {
    let project = TestProject::new("model-ward-comment-string-bait");
    let file = project.main_file(
        r#"
fn bait_case() {
    let _text = "ward Fake { obligation Token must close }";
    // ward CommentOnly { scenario nope {} }
}
"#,
    );
    let output = run_kobo(
        &[s("inspect"), s("--scenario-metadata"), path_arg(&file)],
        &project.root,
    );
    assert_success(&output, "ward scanner should ignore comments and strings");
    assert_not_contains(
        &output.combined(),
        "ward Fake",
        "string contents must not create ward facts",
    );
    assert_not_contains(
        &output.combined(),
        "CommentOnly",
        "comments must not create ward facts",
    );
}

#[test]
fn ward_fact_parser_ignores_nested_comments_and_strings() {
    let project = TestProject::new("model-ward-fact-parser-bait");
    let file = project.main_file(
        r#"
ward ParserBacked {
    state log: Vec<String>
    obligation Delivery must ack

    // scenario comment_only { let delivery = Delivery {}; delivery.ack(); }
    scenario real_case {
        let text = "scenario string_only { let delivery = Delivery {}; }";
        let delivery = Delivery {};
        delivery.ack();
    }
}
"#,
    );
    let output = run_kobo(
        &[s("inspect"), s("--clean"), path_arg(&file)],
        &project.root,
    );
    assert_success(
        &output,
        "ward clean output should ignore nested parser bait",
    );
    assert_contains(
        &output.stdout,
        "fn real_case",
        "real ward scenario should still be generated",
    );
    assert_not_contains(
        &output.stdout,
        "fn comment_only",
        "commented scenario text must not become a generated function",
    );
    assert_not_contains(
        &output.stdout,
        "fn string_only",
        "string scenario text must not become a generated function",
    );
}
