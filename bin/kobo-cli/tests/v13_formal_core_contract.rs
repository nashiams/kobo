mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
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

fn run_witness(project: &TestProject, source: &str, target: &str) -> (PathBuf, Value) {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1303"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(target),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "formal-core fixture should run");
    let witness_path = first_witness_path(project);
    let witness = read_witness(&witness_path);
    (witness_path, witness)
}

fn first_witness_path(project: &TestProject) -> PathBuf {
    project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist")
}

fn read_witness(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("witness should read"))
        .expect("witness should parse")
}

fn core_terminators(witness: &Value) -> Vec<&Value> {
    witness["formal_core"]["functions"]
        .as_array()
        .expect("formal_core.functions should be present")
        .iter()
        .flat_map(|function| {
            function["blocks"]
                .as_array()
                .expect("formal_core function blocks should be present")
        })
        .flat_map(|block| {
            block["terminators"]
                .as_array()
                .expect("formal_core block terminators should be present")
        })
        .collect()
}

fn core_statements(witness: &Value) -> Vec<&Value> {
    witness["formal_core"]["functions"]
        .as_array()
        .expect("formal_core.functions should be present")
        .iter()
        .flat_map(|function| {
            function["blocks"]
                .as_array()
                .expect("formal_core function blocks should be present")
        })
        .flat_map(|block| {
            block["statements"]
                .as_array()
                .expect("formal_core block statements should be present")
        })
        .collect()
}

fn terminator_by_kind<'a>(witness: &'a Value, kind: &str) -> &'a Value {
    core_terminators(witness)
        .into_iter()
        .find(|terminator| terminator["kind"].as_str() == Some(kind))
        .unwrap_or_else(|| {
            panic!(
                "missing Core terminator `{kind}` in {}",
                witness["formal_core"]
            )
        })
}

fn assert_source_span(value: &Value, line: u64, snippet: &str) {
    let span = &value["source_span"];
    assert_eq!(
        span["line"].as_u64(),
        Some(line),
        "Core source span should point to original source line: {value}"
    );
    assert_eq!(
        span["mapped"].as_bool(),
        Some(true),
        "Core source span should be explicitly mapped: {value}"
    );
    assert!(
        span["end"].as_u64().unwrap_or_default() > span["start"].as_u64().unwrap_or_default(),
        "Core source span should have a non-empty byte range: {value}"
    );
    assert_contains(
        span["snippet"].as_str().unwrap_or_default(),
        snippet,
        "Core source span should preserve original source snippet",
    );
}

#[test]
fn question_mark_lowers_to_error_exit_terminator() {
    let project = TestProject::new("v13-core-question");
    let source = r#"
struct Fallible {}

impl Fallible {
    fn run(&self) -> Result<(), ()> { Ok(()) }
}

#[kobo::scenario(profile = "sync")]
fn renamed_question_case() -> Result<(), ()> {
    let tool = Fallible {};
    tool.run()?;
    Ok(())
}
"#;

    let (_path, witness) = run_witness(&project, source, "renamed_question_case");
    let terminator = terminator_by_kind(&witness, "error_exit");
    assert_source_span(terminator, 11, "tool.run()?");
}

#[test]
fn panic_lowers_to_panic_terminator() {
    let project = TestProject::new("v13-core-panic");
    let source = r#"
#[kobo::scenario(profile = "sync")]
fn renamed_panic_case() {
    let message = "panic!(\"string bait\") should not lower";
    // panic!("comment bait") should not lower.
    panic!("real panic edge");
}
"#;

    let (_path, witness) = run_witness(&project, source, "renamed_panic_case");
    let terminator = terminator_by_kind(&witness, "panic");
    assert_source_span(terminator, 6, "panic!");
    let panic_count = core_terminators(&witness)
        .into_iter()
        .filter(|terminator| terminator["kind"].as_str() == Some("panic"))
        .count();
    assert_eq!(
        panic_count, 1,
        "comments and strings must not create extra panic Core edges"
    );
}

#[test]
fn return_lowers_to_return_terminator() {
    let project = TestProject::new("v13-core-return");
    let source = r#"
#[kobo::scenario(profile = "sync")]
fn renamed_return_case() {
    let renamed = 10;
    if renamed > 1 {
        return;
    }
}
"#;

    let (_path, witness) = run_witness(&project, source, "renamed_return_case");
    let terminator = terminator_by_kind(&witness, "return");
    assert_source_span(terminator, 6, "return");
}

#[test]
fn await_lowers_to_resume_and_cancel_edges() {
    let project = TestProject::new("v13-core-await");
    let source = r#"
async fn helper_reordered() {}

#[kobo::scenario(profile = "async")]
async fn renamed_await_case() {
    let bait = ".await in a string";
    // helper_reordered().await in a comment.
    helper_reordered().await;
}
"#;

    let (_path, witness) = run_witness(&project, source, "renamed_await_case");
    let terminator = terminator_by_kind(&witness, "await");
    let edges = terminator["edges"]
        .as_array()
        .expect("await terminator should carry edge names")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(edges.contains(&"await_resume"), "await resume edge missing");
    assert!(edges.contains(&"await_cancel"), "await cancel edge missing");
    assert_source_span(terminator, 8, ".await");
}

#[test]
fn opaque_call_lowers_to_opaque_boundary_edge() {
    let project = TestProject::new("v13-core-opaque");
    let source = r#"
#[kobo::boundary(crate = "live_payments", policy = "opaque", reason = "outside replay")]
use live_payments::Client;

#[kobo::scenario(profile = "sync")]
fn renamed_opaque_case() {
    let _client = Client::new();
}
"#;

    let (_path, witness) = run_witness(&project, source, "renamed_opaque_case");
    let terminator = terminator_by_kind(&witness, "opaque_boundary");
    assert_eq!(terminator["boundary"], "live_payments");
    assert_eq!(terminator["policy"], "opaque");
    assert_eq!(witness["replay_guarantee"], "partial");
    assert_source_span(terminator, 7, "Client::new");
}

#[test]
fn source_spans_survive_kir_to_core_lowering() {
    let project = TestProject::new("v13-core-source-spans");
    let source = r#"
struct Queue {}
struct Parcel {}

impl Queue {
    async fn recv(&mut self) -> Parcel { Parcel {} }
}

impl Parcel {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "async")]
async fn source_span_case() {
    let mut inbox = Queue {};
    let parcel = inbox.recv().await;
    parcel.ack();
}
"#;

    let (_path, witness) = run_witness(&project, source, "source_span_case");
    let create = core_statements(&witness)
        .into_iter()
        .find(|statement| statement["kind"].as_str() == Some("obligation_create"))
        .expect("queue receive should create a Core obligation");
    assert_eq!(create["binding"], "parcel");
    assert_source_span(create, 18, "inbox.recv().await");
    let discharge = core_statements(&witness)
        .into_iter()
        .find(|statement| statement["kind"].as_str() == Some("obligation_discharge"))
        .expect("parcel ack should discharge a Core obligation");
    assert_source_span(discharge, 19, "parcel.ack()");
}

#[test]
fn invalid_unmapped_core_node_fails_instead_of_inventing_span() {
    let project = TestProject::new("v13-core-unmapped");
    let source = r#"
#[kobo::scenario(profile = "sync")]
fn unmapped_case() {
    return;
}
"#;

    let (witness_path, mut witness) = run_witness(&project, source, "unmapped_case");
    witness["formal_core"]["functions"][0]["blocks"][0]["terminators"][0]["source_span"]
        ["mapped"] = Value::Bool(false);
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(&witness).expect("witness should serialize"),
    )
    .expect("mutated witness should write");

    let output = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--roundtrip-metadata"),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(&output, "unmapped Core nodes must fail witness validation");
    assert_contains(
        &output.combined(),
        "unmapped Core node",
        "replay validation should name the source-map failure",
    );
}

#[test]
fn comments_strings_and_names_do_not_affect_core_obligations() {
    let project = TestProject::new("v13-core-text-bait");
    let source = r#"
#[kobo::scenario(profile = "sync")]
fn text_bait_case() {
    let error_exit = "? should not lower from this string";
    let panic_name = "panic!(\"bait\")";
    let await_name = ".await";
    // return; panic!("bait"); helper().await?;
    let _renamed = (error_exit, panic_name, await_name);
}
"#;

    let (_path, witness) = run_witness(&project, source, "text_bait_case");
    let kinds = core_terminators(&witness)
        .into_iter()
        .filter_map(|terminator| terminator["kind"].as_str())
        .collect::<Vec<_>>();
    assert!(
        !kinds
            .iter()
            .any(|kind| matches!(*kind, "error_exit" | "panic" | "await" | "opaque_boundary")),
        "comments, strings, and renamed variables must not create Core edges: {kinds:?}"
    );
}

#[test]
fn kwit_records_template_ids_versions_and_obligation_states() {
    let project = TestProject::new("v13-core-template-evidence");
    let source = r#"
struct Queue {}
struct Parcel {}

impl Queue {
    async fn recv(&mut self) -> Parcel { Parcel {} }
}

impl Parcel {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "async")]
async fn evidence_case() {
    let mut inbox = Queue {};
    let delivery = inbox.recv().await;
    delivery.requeue();
}
"#;

    let (_path, witness) = run_witness(&project, source, "evidence_case");
    let obligations = witness["lifecycle_inference"]["obligations"]
        .as_array()
        .expect("lifecycle inference obligations should be present");
    let delivery = obligations
        .iter()
        .find(|obligation| obligation["binding"].as_str() == Some("delivery"))
        .expect("delivery lifecycle evidence should exist");
    assert_eq!(delivery["template_id"], "queue_delivery");
    assert_eq!(delivery["template_version"], "v0.13.0");
    assert_eq!(delivery["state"], "discharged");
    assert_eq!(delivery["source_span"]["mapped"], Value::Bool(true));
}

#[test]
fn summary_records_core_and_template_facts() {
    let project = TestProject::new("v13-core-summary-evidence");
    let source = r#"
#[kobo::scenario(profile = "sync")]
fn summary_case() {
    return;
}
"#;

    let (_path, witness) = run_witness(&project, source, "summary_case");
    assert_eq!(witness["formal_core"]["source"], "compiler_core_ir");
    assert!(
        witness["summaries"]
            .as_array()
            .expect("summaries should be array")
            .iter()
            .any(|summary| summary.to_string().contains("formal_core")),
        "summary evidence should mention formal_core facts: {}",
        witness["summaries"]
    );
}

#[test]
fn proof_seed_records_core_edge_and_obligation_exit_state() {
    let project = TestProject::new("v13-core-proof-seed");
    let source = r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "sync")]
fn proof_seed_case() {
    let renamed = Delivery {};
    renamed.ack();
    return;
}
"#;

    let (_path, witness) = run_witness(&project, source, "proof_seed_case");
    let proof_seed = &witness["proof_seed"];
    assert_eq!(
        proof_seed["theorem_target"],
        "no_unresolved_local_obligation_on_modeled_exit"
    );
    assert!(
        proof_seed["core_edges"]
            .as_array()
            .expect("proof seed should record Core edges")
            .iter()
            .any(|edge| edge["kind"].as_str() == Some("return")),
        "proof seed should record the return edge: {proof_seed}"
    );
    assert!(
        proof_seed["modeled_exit_obligation_states"]
            .as_array()
            .expect("proof seed should record obligation states")
            .iter()
            .any(|state| state["state"].as_str() == Some("discharged")),
        "proof seed should record discharged obligation state: {proof_seed}"
    );
}

#[test]
fn opaque_and_debt_boundaries_downgrade_exact_replay() {
    let project = TestProject::new("v13-core-boundary-downgrade");
    let source = r#"
#[kobo::boundary(crate = "live_search", policy = "opaque", reason = "outside deterministic replay")]
use live_search::Client;

#[kobo::scenario(profile = "sync")]
fn downgrade_case() {
    let _client = Client::new();
}
"#;

    let (_path, witness) = run_witness(&project, source, "downgrade_case");
    assert_eq!(witness["replay_guarantee"], "partial");
    assert_eq!(witness["replay_grade"], "partial");
    assert!(
        witness["boundary_ledger"]
            .as_array()
            .expect("boundary ledger should exist")
            .iter()
            .any(|entry| entry["boundary"].as_str() == Some("live_search")
                && entry["status"].as_str() == Some("opaque")),
        "opaque boundary should be represented as policy evidence: {}",
        witness["boundary_ledger"]
    );
}

#[test]
fn branch_terminator_records_distinct_core_targets() {
    let project = TestProject::new("v13-core-branch-targets");
    let source = r#"
fn runtime_flag(input: i32) -> bool { input > 0 }

#[kobo::scenario(profile = "sync")]
fn branch_target_case() {
    if runtime_flag(1) {
        ward.task();
    } else {
        ward.random.u64();
    }
}
"#;

    let (_path, witness) = run_witness(&project, source, "branch_target_case");
    let branch = terminator_by_kind(&witness, "branch");
    let edges = branch["edges"]
        .as_array()
        .expect("branch terminator should carry Core edge names")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        edges.len(),
        2,
        "two-arm source branch should preserve two Core edges: {branch}"
    );
    assert_ne!(
        edges[0], edges[1],
        "branch arms should target distinct Core blocks instead of duplicating a linear next edge: {branch}"
    );
    assert!(
        edges.iter().all(|edge| edge.starts_with("goto:bb")),
        "branch edges should name Core block targets: {edges:?}"
    );
}

#[test]
fn formal_core_is_compiler_owned_cfg_not_cli_text_scan() {
    let project = TestProject::new("v13-core-compiler-owned-cfg");
    let source = r#"
#[kobo::must_call(done)]
struct Token {}

impl Token {
    fn done(self) {}
}

#[kobo::scenario(profile = "sync")]
fn compiler_owned_cfg_case() {
    let token = Token {};
    token.done();
    let renamed = 10;
    if renamed > 1 {
        return;
    }
}
"#;

    let (_path, witness) = run_witness(&project, source, "compiler_owned_cfg_case");
    assert_eq!(witness["formal_core"]["source"], "compiler_core_ir");
    assert_eq!(
        witness["formal_core"]["cfg_source"], "kir_cfg",
        "formal Core should record that successor shape came from KIR CFG facts"
    );
    let blocks = witness["formal_core"]["functions"][0]["blocks"]
        .as_array()
        .expect("Core function should expose CFG blocks");
    assert!(
        blocks.len() > 1,
        "compiler Core should expose block-level CFG evidence, not a single entry bucket: {blocks:?}"
    );
    assert!(
        blocks
            .iter()
            .any(|block| block["successors"].as_array().is_some()),
        "Core blocks should carry successor evidence: {blocks:?}"
    );
    assert!(
        blocks.iter().any(|block| {
            block["successor_source"].as_str() == Some("kir_cfg")
                && block["kir_cfg_block"].as_u64().is_some()
        }),
        "Core blocks should retain their originating KIR CFG block IDs: {blocks:?}"
    );
}

#[test]
fn declared_ack_nack_requeue_is_not_guessed_as_queue_delivery() {
    let project = TestProject::new("v13-core-declared-template");
    let source = r#"
#[kobo::must_call(ack | nack | requeue)]
struct ManualToken {}

impl ManualToken {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "sync")]
fn declared_template_case() {
    let token = ManualToken {};
    token.ack();
}
"#;

    let (_path, witness) = run_witness(&project, source, "declared_template_case");
    let obligations = witness["lifecycle_inference"]["obligations"]
        .as_array()
        .expect("lifecycle inference obligations should be present");
    let token = obligations
        .iter()
        .find(|obligation| obligation["binding"].as_str() == Some("token"))
        .expect("manual token lifecycle evidence should exist");
    assert_ne!(
        token["template_id"], "queue_delivery",
        "manual must_call declarations must not be relabeled as inferred queue templates: {token}"
    );
    assert_eq!(token["template_source"], "declaration");
    assert_eq!(token["kind"], "declared_must_call");
}
