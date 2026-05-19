mod v09_common;

use std::fs;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_success, path_arg, run_kobo, s, unique_symbol, TestProject,
};

#[test]
fn explicit_opaque_boundary_downgrades_to_partial_with_replay_grade_and_ledger() {
    let project = TestProject::new("v10-opaque-boundary-ledger");
    let crate_name = unique_symbol("live_http");
    let file = project.main_file(&format!(
        r#"
#[kobo::boundary(crate = "{crate_name}", policy = "opaque", reason = "live HTTP remains outside replay")]
use {crate_name}::Client;

#[kobo::scenario(profile = "async")]
fn live_gateway() {{
    let _client = Client::new();
    ward.task();
}}
"#
    ));

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("101"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "explicit opaque boundary should produce a partial witness instead of failing",
    );
    let witness = read_first_witness(&project);
    assert_eq!(witness["replay_grade"], "partial");
    assert_eq!(witness["replay_guarantee"], "partial");
    assert_eq!(witness["execution_digest"]["harness_engine"], "none");

    let ledger = witness["boundary_ledger"]
        .as_array()
        .expect("boundary ledger should be an array");
    let opaque = ledger
        .iter()
        .find(|entry| entry["boundary"] == crate_name)
        .unwrap_or_else(|| panic!("opaque boundary missing from ledger: {ledger:?}"));
    assert_eq!(opaque["status"], "opaque");
    assert_eq!(opaque["policy"], "opaque");
    assert_contains(
        &opaque.to_string(),
        "live HTTP remains outside replay",
        "ledger should preserve the source policy reason",
    );
}

#[test]
fn lifecycle_observation_reports_inferred_obligations_from_semantics_not_text_bait() {
    let project = TestProject::new("v10-lifecycle-inference");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn renamed_delivery_flow() {
    // let phantom = Delivery {}; phantom.ack();
    let bait = "Delivery should not be inferred from this string: bait.ack()";
    let renamed_delivery = Delivery {};
    renamed_delivery.ack();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("102"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "discharged delivery should compile while reporting lifecycle inference evidence",
    );
    let witness = read_first_witness(&project);
    assert_eq!(witness["lifecycle_inference"]["mode"], "observe");
    let inferred = witness["inferred_obligations"]
        .as_array()
        .expect("inferred obligations should be an array");
    assert_eq!(
        inferred.len(),
        1,
        "comments and strings must not create extra lifecycle obligations: {inferred:?}"
    );
    let obligation = &inferred[0];
    assert_eq!(obligation["kind"], "queue_delivery");
    assert_eq!(obligation["template_id"], "queue_delivery");
    assert_eq!(obligation["template_version"], "v0.10.1");
    assert_eq!(obligation["binding"], "renamed_delivery");
    assert_eq!(obligation["state"], "discharged");
    assert_eq!(obligation["confidence"], "exact_template");
    assert_eq!(obligation["coverage_loss"], Value::Null);
    assert_contains(
        &obligation["terminal_actions"].to_string(),
        "requeue",
        "queue delivery template should retain all terminal actions",
    );
    assert_contains(
        &witness["call_graph_obligation_summaries"].to_string(),
        "renamed_delivery",
        "call-graph obligation summaries should include lifecycle facts",
    );
}

#[test]
fn normal_code_lifecycle_templates_are_inferred_without_manual_must_call_attrs() {
    let project = TestProject::new("v10-normal-lifecycle-inference");
    let file = project.main_file(
        r#"
struct Queue {}
struct Delivery {}
struct Db {}
struct Transaction {}

impl Queue {
    async fn recv(&self) -> Delivery {
        Delivery {}
    }
}

impl Db {
    fn begin(&self) -> Transaction {
        Transaction {}
    }
}

#[kobo::scenario(profile = "async")]
async fn service() {
    let message = Queue {}.recv().await;
    message.ack();

    let tx = Db {}.begin();
    tx.commit();

    let task = tokio::spawn(async {});
    task.abort();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("103"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "normal Rust-shaped lifecycle code should infer obligations without manual must_call attrs",
    );
    let witness = read_first_witness(&project);
    let inferred = witness["inferred_obligations"]
        .as_array()
        .expect("inferred obligations should be an array");
    let kinds = inferred
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap_or("<missing>"))
        .collect::<Vec<_>>();
    for kind in ["queue_delivery", "transaction", "spawned_task"] {
        assert!(
            kinds.contains(&kind),
            "missing inferred lifecycle kind `{kind}` in {inferred:?}"
        );
    }
    assert!(
        inferred
            .iter()
            .all(|entry| entry["state"] == "discharged" && entry["confidence"] == "exact_template"),
        "normal lifecycle facts should be discharged exact-template observations: {inferred:?}"
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
