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

fn run_witness(project: &TestProject, source: &str, target: &str) -> Value {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("1301"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(target),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "v0.13 protocol-template fixture should run");
    first_witness(project)
}

fn run_failing_witness(project: &TestProject, source: &str, target: &str) -> Value {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("1302"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(target),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "unresolved protocol-template fixture should fail and emit a witness",
    );
    first_witness(project)
}

fn first_witness(project: &TestProject) -> Value {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

fn inferred_obligations(witness: &Value) -> &[Value] {
    witness["inferred_obligations"]
        .as_array()
        .expect("inferred obligations should be present")
}

fn obligation_by_template<'a>(witness: &'a Value, template_id: &str) -> &'a Value {
    inferred_obligations(witness)
        .iter()
        .find(|entry| entry["template_id"].as_str() == Some(template_id))
        .unwrap_or_else(|| {
            panic!(
                "missing inferred obligation template `{template_id}` in {}",
                witness["inferred_obligations"]
            )
        })
}

fn assert_terminal_actions(obligation: &Value, expected: &[&str]) {
    let actions = obligation["terminal_actions"]
        .as_array()
        .expect("terminal_actions should be array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    for expected_action in expected {
        assert!(
            actions.contains(expected_action),
            "missing terminal action `{expected_action}` in {actions:?}"
        );
    }
}

fn assert_source_span(obligation: &Value, expected_line: u64) {
    assert_eq!(
        obligation["source_span"]["line"].as_u64(),
        Some(expected_line),
        "template fact should point to the source create site: {obligation}"
    );
    assert!(
        obligation["source_span"]["start"].as_u64().is_some(),
        "template fact should carry a byte span: {obligation}"
    );
}

#[test]
fn queue_delivery_infers_ack_nack_requeue_without_manual_declaration() {
    let project = TestProject::new("v13-template-queue");
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
async fn renamed_queue_case() {
    let mut inbox = Queue {};
    let parcel = inbox.recv().await;
    parcel.ack();
}
"#;

    let witness = run_witness(&project, source, "renamed_queue_case");
    let obligation = obligation_by_template(&witness, "queue_delivery");
    assert_eq!(obligation["state"], "discharged");
    assert_terminal_actions(obligation, &["ack", "nack", "requeue"]);
    assert_source_span(obligation, 18);
}

#[test]
fn transaction_infers_commit_rollback_without_manual_declaration() {
    let project = TestProject::new("v13-template-transaction");
    let source = r#"
struct Database {}
struct UnitOfWork {}

impl Database {
    fn begin(&self) -> UnitOfWork { UnitOfWork {} }
}

impl UnitOfWork {
    fn commit(self) {}
    fn rollback(self) {}
}

#[kobo::scenario(profile = "sync")]
fn reordered_transaction_case() {
    let store = Database {};
    let unit = store.begin();
    unit.rollback();
}
"#;

    let witness = run_witness(&project, source, "reordered_transaction_case");
    let obligation = obligation_by_template(&witness, "transaction");
    assert_eq!(obligation["state"], "discharged");
    assert_terminal_actions(obligation, &["commit", "rollback"]);
    assert_source_span(obligation, 17);
}

#[test]
fn handler_reply_infers_reply_reject_cancel_without_manual_declaration() {
    let project = TestProject::new("v13-template-handler");
    let source = r#"
struct RequestToken {}

impl RequestToken {
    fn reply(self) {}
    fn reject(self) {}
    fn cancel(self) {}
}

#[kobo::handler]
async fn gateway_handler(token: RequestToken) {
    token.reject();
}

#[kobo::scenario(profile = "async")]
async fn gateway_case() {
    gateway_handler(RequestToken {}).await;
}
"#;

    let witness = run_witness(&project, source, "gateway_case");
    let obligation = obligation_by_template(&witness, "handler_reply");
    assert_eq!(obligation["state"], "discharged");
    assert_terminal_actions(obligation, &["reply", "reject", "cancel"]);
    assert_source_span(obligation, 11);
}

#[test]
fn spawned_task_infers_await_abort_detach_policy() {
    let project = TestProject::new("v13-template-spawned-task");
    let source = r#"
#[kobo::scenario(profile = "async")]
async fn task_case() {
    let worker = tokio::spawn(async {});
    worker.abort();
}
"#;

    let witness = run_witness(&project, source, "task_case");
    let obligation = obligation_by_template(&witness, "spawned_task");
    assert_eq!(obligation["state"], "discharged");
    assert_terminal_actions(obligation, &["await", "abort", "detach-with-policy"]);
    assert_source_span(obligation, 4);
}

#[test]
fn lock_permit_infers_release_or_safe_drop_boundary() {
    let project = TestProject::new("v13-template-lock");
    let source = r#"
struct Gate {}
struct Permit {}

impl Gate {
    async fn lock(&self) -> Permit { Permit {} }
}

impl Permit {
    fn release(self) {}
}

#[kobo::scenario(profile = "async")]
async fn lock_case() {
    let gate = Gate {};
    let guard = gate.lock().await;
    guard.release();
}
"#;

    let witness = run_witness(&project, source, "lock_case");
    let obligation = obligation_by_template(&witness, "lock_permit");
    assert_eq!(obligation["state"], "discharged");
    assert_terminal_actions(obligation, &["release", "drop-at-safe-boundary"]);
    assert_source_span(obligation, 16);
}

#[test]
fn file_socket_infers_close_transfer_or_boundary() {
    let project = TestProject::new("v13-template-file-socket");
    let source = r#"
struct Listener {}
struct Connection {}

impl Listener {
    async fn accept(&self) -> Connection { Connection {} }
}

impl Connection {
    fn close(self) {}
}

#[kobo::scenario(profile = "network")]
async fn socket_case() {
    let listener = Listener {};
    let channel = listener.accept().await;
    channel.close();
}
"#;

    let witness = run_witness(&project, source, "socket_case");
    let obligation = obligation_by_template(&witness, "file_socket");
    assert_eq!(obligation["state"], "discharged");
    assert_terminal_actions(obligation, &["close", "transfer", "opaque-boundary"]);
    assert_source_span(obligation, 16);
}

#[test]
fn template_facts_are_structured_data_not_diagnostic_text() {
    let project = TestProject::new("v13-template-structured-data");
    let source = r#"
struct Queue {}
struct Parcel {}

impl Queue {
    async fn recv(&self) -> Parcel { Parcel {} }
}

impl Parcel {
    fn requeue(self) {}
}

#[kobo::scenario(profile = "async")]
async fn structured_case() {
    let delivery = Queue {}.recv().await;
    delivery.requeue();
}
"#;

    let witness = run_witness(&project, source, "structured_case");
    let obligation = obligation_by_template(&witness, "queue_delivery");
    for field in [
        "id",
        "template_id",
        "template_version",
        "kind",
        "binding",
        "state",
        "terminal_actions",
        "source_span",
        "confidence",
        "coverage_loss",
    ] {
        assert!(
            obligation.get(field).is_some(),
            "structured template fact missing `{field}`: {obligation}"
        );
    }
    assert_eq!(obligation["template_version"], "v0.13.0");
    assert_eq!(obligation["confidence"], "exact_template");
    assert!(
        obligation["source_span"].is_object(),
        "source_span must be structured JSON, not text: {obligation}"
    );
}

#[test]
fn unknown_helper_becomes_transfer_debt_or_escape_not_discharge() {
    let project = TestProject::new("v13-template-helper-transfer");
    let source = r#"
struct Queue {}
struct Parcel {}

impl Queue {
    async fn recv(&self) -> Parcel { Parcel {} }
}

impl Parcel {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

fn helper_before_scenario(input: Parcel) {
    external_boundary(input);
}

fn external_boundary<T>(_value: T) {}

#[kobo::scenario(profile = "async")]
async fn helper_transfer_case() {
    let renamed = Queue {}.recv().await;
    helper_before_scenario(renamed);
}
"#;

    let witness = run_failing_witness(&project, source, "helper_transfer_case");
    let obligation = obligation_by_template(&witness, "queue_delivery");
    assert_ne!(
        obligation["state"], "discharged",
        "unknown helper transfer must not count as discharge: {obligation}"
    );
    assert_contains(
        &witness["operation_coverage"].to_string(),
        "obligation-transfer",
        "helper call should be represented as a transfer/debt fact",
    );
}

#[test]
fn method_name_alone_does_not_infer_lifecycle_template() {
    let project = TestProject::new("v13-template-no-method-name-only");
    let source = r#"
struct Inbox {}
struct PlainMessage {}

impl Inbox {
    async fn recv(&self) -> PlainMessage { PlainMessage {} }
}

impl PlainMessage {
    fn store(self) {}
}

#[kobo::scenario(profile = "async")]
async fn unrelated_recv_case() {
    let inbox = Inbox {};
    let message = inbox.recv().await;
    message.store();
}
"#;

    let witness = run_witness(&project, source, "unrelated_recv_case");
    assert!(
        inferred_obligations(&witness).is_empty(),
        "method name alone must not infer queue_delivery: {}",
        witness["inferred_obligations"]
    );
}

#[test]
fn unrelated_method_name_does_not_discharge_obligation_token() {
    let project = TestProject::new("v13-template-terminal-action-shape");
    let source = r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {}

impl Transaction {
    fn ack(&self) {}
}

#[kobo::scenario(profile = "sync")]
fn unrelated_ack_case() {
    let tx = Transaction {};
    tx.ack();
}
"#;

    let witness = run_failing_witness(&project, source, "unrelated_ack_case");
    let obligation = obligation_by_template(&witness, "declared_must_call:Transaction");
    assert_ne!(
        obligation["state"], "discharged",
        "only declared terminal actions may discharge the obligation: {obligation}"
    );
}

#[test]
fn comments_strings_and_variable_names_do_not_create_or_discharge_obligations() {
    let project = TestProject::new("v13-template-anti-gaming");
    let source = r#"
#[kobo::scenario(profile = "sync")]
fn anti_gaming_case() {
    // queue.recv().await should not create an obligation from a comment.
    let delivery_named_for_the_test = "ack should not discharge from a string";
    let _copy = delivery_named_for_the_test;
}
"#;

    let witness = run_witness(&project, source, "anti_gaming_case");
    assert!(
        inferred_obligations(&witness).is_empty(),
        "comments, strings, and variable names must not create lifecycle facts: {}",
        witness["inferred_obligations"]
    );
}
