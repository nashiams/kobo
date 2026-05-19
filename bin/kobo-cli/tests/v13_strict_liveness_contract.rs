mod v09_common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn run_strict_witness(project: &TestProject, source: &str, target: &str) -> (CliOutput, Value) {
    run_strict_witness_with_args(project, source, target, &[])
}

fn run_strict_witness_with_args(
    project: &TestProject,
    source: &str,
    target: &str,
    extra_args: &[String],
) -> (CliOutput, Value) {
    let file = project.main_file(source);
    let mut args = vec![
        s("test"),
        s("--sim"),
        s("quick"),
        s("--profile"),
        s("release"),
        s("--engine"),
        s("semantic"),
        s("--seed"),
        s("1304"),
        s("--witness-dir"),
        s(".kobo/witnesses"),
        s("--target"),
        s(target),
        s("--error-format=json"),
    ];
    args.extend_from_slice(extra_args);
    args.push(path_arg(&file));

    let output = run_kobo(&args, &project.root);
    let witness = first_witness(project).unwrap_or_else(|| Value::Null);
    (output, witness)
}

fn first_witness(project: &TestProject) -> Option<Value> {
    let witness_path = project.find_files_with_ext("kwit").into_iter().next()?;
    Some(read_witness(&witness_path))
}

fn read_witness(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("witness should read"))
        .expect("witness should parse")
}

fn strict_errors(witness: &Value) -> &[Value] {
    witness["strict_liveness"]["errors"]
        .as_array()
        .expect("strict_liveness.errors should exist")
}

fn assert_strict_error(witness: &Value, exit_kind: &str, binding: &str) {
    assert!(
        strict_errors(witness).iter().any(|error| {
            error["exit_kind"].as_str() == Some(exit_kind)
                && error["binding"].as_str() == Some(binding)
        }),
        "missing strict liveness error `{exit_kind}` for `{binding}` in {}",
        witness["strict_liveness"]
    );
}

fn delivery_source(body: &str) -> String {
    format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

{body}
"#
    )
}

#[test]
fn strict_errors_on_undischarged_must_call_path() {
    let project = TestProject::new("v13-strict-undischarged");
    let source = delivery_source(
        r#"
#[kobo::scenario(profile = "sync")]
fn dropped_delivery_case() {
    let renamed = Delivery {};
    let _lost = renamed;
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "dropped_delivery_case");
    assert_failure(&output, "strict liveness should reject dropped delivery");
    assert_contains(&output.combined(), "K0100", "strict error should use K0100");
    assert_eq!(witness["strict_liveness"]["status"], "failed");
    assert_strict_error(&witness, "normal_exit", "renamed");
}

#[test]
fn strict_accepts_discharge_return_transfer_escape_and_reasoned_suppression() {
    let project = TestProject::new("v13-strict-accepted-forms");
    let source = delivery_source(
        r#"
#[kobo::boundary(crate = "handoff", policy = "opaque", reason = "owned by external owner")]
use handoff::Sink;

fn helper_finish(token: Delivery) {
    token.ack();
}

#[kobo::scenario(profile = "sync")]
fn accepted_forms_case() -> Delivery {
    let direct = Delivery {};
    direct.ack();

    let transferred = Delivery {};
    helper_finish(transferred);

    let escaped = Delivery {};
    Sink::store(escaped);

    #[kobo::suppress_liveness(reason = "tracked by outer owner")]
    let suppressed = Delivery {};

    let returned = Delivery {};
    return returned;
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "accepted_forms_case");
    assert_success(
        &output,
        "strict liveness should accept all reasoned resolution forms",
    );
    assert_eq!(witness["strict_liveness"]["status"], "passed");
    assert!(
        witness["strict_liveness"]["reasoned_suppressions"]
            .as_array()
            .expect("strict liveness should expose suppression ledger")
            .iter()
            .any(|entry| entry["binding"].as_str() == Some("suppressed")
                && entry["reason"].as_str() == Some("tracked by outer owner")),
        "reasoned suppression should be visible in evidence: {}",
        witness["strict_liveness"]
    );
    assert!(
        witness["strict_liveness"]["resolved_paths"]
            .as_array()
            .expect("strict liveness should expose resolved paths")
            .iter()
            .any(|entry| entry["resolution"].as_str() == Some("escaped")),
        "explicit boundary escape should be visible in evidence: {}",
        witness["strict_liveness"]
    );
}

#[test]
fn question_mark_with_unresolved_obligation_fails() {
    let project = TestProject::new("v13-strict-question");
    let source = delivery_source(
        r#"
fn fallible() -> Result<(), ()> { Ok(()) }

#[kobo::scenario(profile = "sync")]
fn question_case() -> Result<(), ()> {
    let token = Delivery {};
    fallible()?;
    token.ack();
    Ok(())
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "question_case");
    assert_failure(&output, "unresolved token at ? must fail");
    assert_strict_error(&witness, "error_exit", "token");
    assert_contains(
        &witness["strict_liveness"].to_string(),
        "fallible()?",
        "strict failure should carry the source-mapped ? edge",
    );
}

#[test]
fn panic_with_unresolved_obligation_fails() {
    let project = TestProject::new("v13-strict-panic");
    let source = delivery_source(
        r#"
#[kobo::scenario(profile = "sync")]
fn panic_case() {
    let token = Delivery {};
    panic!("boom before ack");
    token.ack();
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "panic_case");
    assert_failure(&output, "unresolved token at panic must fail");
    assert_strict_error(&witness, "panic", "token");
}

#[test]
fn cancel_edge_without_handler_fails() {
    let project = TestProject::new("v13-strict-cancel");
    let source = delivery_source(
        r#"
#[kobo::scenario(profile = "async")]
fn cancel_case() {
    let token = Delivery {};
    ward.task();
    token.ack();
}
"#,
    );

    let (output, witness) = run_strict_witness_with_args(
        &project,
        &source,
        "cancel_case",
        &[s("--inject"), s("cancel")],
    );
    assert_failure(&output, "cancel edge must reject unresolved local token");
    assert_strict_error(&witness, "cancel", "token");
}

#[test]
fn aliasing_and_shadowing_track_same_obligation_token() {
    let project = TestProject::new("v13-strict-alias-shadow");
    let source = delivery_source(
        r#"
#[kobo::scenario(profile = "sync")]
fn alias_shadow_case() {
    let token = Delivery {};
    let alias = token;
    let token = 42;
    let _shadow = token;
    alias.ack();
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "alias_shadow_case");
    assert_success(
        &output,
        "alias discharge should resolve the original obligation despite shadowing",
    );
    assert_eq!(witness["strict_liveness"]["status"], "passed");
}

#[test]
fn helper_summary_proves_discharge_without_manual_annotation() {
    let project = TestProject::new("v13-strict-helper");
    let source = delivery_source(
        r#"
fn helper_reordered(token: Delivery) {
    token.requeue();
}

#[kobo::scenario(profile = "sync")]
fn helper_case() {
    let delivery = Delivery {};
    helper_reordered(delivery);
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "helper_case");
    assert_success(&output, "helper summary should prove discharge");
    let summaries = witness["call_graph_obligation_summaries"]["summaries"]
        .as_array()
        .expect("helper summary evidence should be present");
    assert!(
        summaries
            .iter()
            .any(|summary| summary["discharges"].to_string().contains("delivery")),
        "helper discharge should be visible in summaries: {summaries:?}"
    );
}

#[test]
fn unknown_recursion_scc_conservatively_escapes_or_fails() {
    let project = TestProject::new("v13-strict-recursive-scc");
    let source = delivery_source(
        r#"
fn recursive_helper(token: Delivery, depth: bool) {
    if depth {
        recursive_helper(token, false);
    }
}

#[kobo::scenario(profile = "sync")]
fn recursive_case() {
    let delivery = Delivery {};
    recursive_helper(delivery, true);
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "recursive_case");
    assert_failure(
        &output,
        "recursive SCC without proof should fail instead of assuming discharge",
    );
    assert!(
        witness["call_graph_obligation_summaries"]["sccs"]
            .as_array()
            .expect("SCC evidence should exist")
            .iter()
            .any(|scc| scc["is_recursive"].as_bool() == Some(true)),
        "recursive SCC should be visible in evidence: {}",
        witness["call_graph_obligation_summaries"]
    );
}

#[test]
fn arc_mutex_delivery_fails_without_obligation_aware_wrapper_or_declaration() {
    let project = TestProject::new("v13-strict-arc-mutex");
    let source = delivery_source(
        r#"
use std::sync::{Arc, Mutex};

#[kobo::scenario(profile = "sync")]
fn arc_mutex_case() {
    let _shared = Arc::new(Mutex::new(Delivery {}));
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "arc_mutex_case");
    assert_failure(
        &output,
        "Arc<Mutex<Delivery>> must not hide an unresolved obligation",
    );
    assert_contains(
        &witness["strict_liveness"].to_string(),
        "Arc<Mutex",
        "shared-container failure should name the unsupported container",
    );
}

#[test]
fn opaque_boundary_exit_with_unresolved_obligation_fails() {
    let project = TestProject::new("v13-strict-opaque-exit");
    let source = delivery_source(
        r#"
#[kobo::boundary(crate = "live_payments", policy = "opaque", reason = "outside replay")]
use live_payments::Client;

#[kobo::scenario(profile = "sync")]
fn opaque_exit_case() {
    let token = Delivery {};
    let _client = Client::new();
    token.ack();
}
"#,
    );

    let (output, witness) = run_strict_witness(&project, &source, "opaque_exit_case");
    assert_failure(
        &output,
        "opaque boundary edge must reject unresolved local obligations",
    );
    assert_strict_error(&witness, "opaque_boundary", "token");
}

#[test]
fn mode_invariant_preserves_runtime_output_for_accepted_code() {
    let project = TestProject::new("v13-strict-mode-invariant");
    let file = project.main_file(
        r#"
fn main() {
    println!("same runtime behavior");
}
"#,
    );
    let mut outputs = Vec::new();
    for profile in ["dev", "checked", "release"] {
        let output = run_kobo(
            &[s("run"), s("--profile"), s(profile), path_arg(&file)],
            &project.root,
        );
        assert_success(&output, "accepted ordinary code should run in every mode");
        outputs.push(output.stdout);
    }
    assert_eq!(
        outputs[0], outputs[1],
        "dev and checked output should match"
    );
    assert_eq!(
        outputs[1], outputs[2],
        "checked and strict/release output should match"
    );
}
