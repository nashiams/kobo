mod v09_common;

use std::fs;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo, s,
    TestProject,
};

fn handler_fixture() -> (TestProject, std::path::PathBuf) {
    let project = TestProject::new("handler-lifecycle");
    let file = project.main_file(
        r#"
struct Request {
    id: u64,
}

struct Response {
    status: u16,
}

struct HandlerError {}

impl Request {
    fn reply(&self) {}
    fn reject(&self) {}
    fn cancel(&self) {}
}

async fn cleanup_request() {}

#[kobo::handler]
#[kobo::cleanup(cleanup_request)]
async fn handle(request: Request, should_fail: bool) -> Result<Response, HandlerError> {
    if should_fail {
        request.reject();
        return Err(HandlerError {});
    }

    request.reply();
    Ok(Response { status: 200 })
}

#[kobo::scenario(profile = "network")]
async fn handler_success_path() {
    let request = Request { id: 1 };
    ward.network.send("client");
    handle(request, false).await;
    ward.network.receive("client");
}

#[kobo::scenario(profile = "network")]
async fn handler_disconnect_path() {
    let request = Request { id: 2 };
    ward.network.send("client");
    ward.network.drop_message("client");
    handle(request, false).await;
}
"#,
    );
    (project, file)
}

fn inspect_handler() -> String {
    let (project, file) = handler_fixture();
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(&output, "handler fixture should inspect");
    output.combined()
}

fn first_witness(project: &TestProject) -> Value {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("handler simulation should write a witness");
    serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

#[test]
fn handler_auto_wraps_reply_and_reject_results() {
    let generated = inspect_handler();

    for expected in [
        "enum KoboHandlerOutcome",
        "KoboHandlerOutcome::reply",
        "KoboHandlerOutcome::reject",
        "record_reply",
        "record_reject",
    ] {
        assert_contains(
            &generated,
            expected,
            "handler lowering should make reply/reject result wrapping inspectable",
        );
    }
}

#[test]
fn handler_closes_tracing_span_on_drop_path() {
    let generated = inspect_handler();

    for expected in [
        "struct KoboHandlerLifecycleMetrics",
        "struct KoboHandlerLifecycleGuard",
        "impl Drop for KoboHandlerLifecycleGuard",
        "close_tracing_span",
        "metrics_boundary",
        "kobo: handler handle source_line=20",
    ] {
        assert_contains(
            &generated,
            expected,
            "handler lowering should expose tracing and metrics lifecycle boundaries",
        );
    }
}

#[test]
fn handler_runs_cleanup_hook_on_success_error_and_cancel() {
    let generated = inspect_handler();

    for expected in [
        "type KoboHandlerCleanupFuture",
        "tokio::runtime::Builder::new_current_thread()",
        "register_cleanup",
        "|| -> KoboHandlerCleanupFuture",
        "run_registered_cleanup(\"success\").await",
        "run_registered_cleanup(\"error\").await",
        "record_error_boundary(\"handle\"",
        "run_cancel_cleanup_on_drop",
        "KoboHandlerCleanupRuntime::run(cleanup);",
        "record_cleanup_run",
    ] {
        assert_contains(
            &generated,
            expected,
            "handler cleanup hook should be visible on success, error, and cancellation paths",
        );
    }
    assert_not_contains(
        &generated,
        "RawWaker",
        "handler cleanup fallback should use a real runtime instead of a raw no-op waker loop",
    );
    assert_not_contains(
        &generated,
        "thread::yield_now",
        "handler cleanup fallback should not spin-yield while polling cleanup futures",
    );
}

#[test]
fn handler_reports_request_state_leak_with_source_span() {
    let project = TestProject::new("handler-state-leak");
    let source = r#"
struct Request {
    id: u64,
}

#[kobo::handler]
async fn leak_request(request: Request) {
    tokio::spawn(async move {
        let _id = request.id;
    });
}
"#;
    let file = project.main_file(source);
    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );

    assert_failure(
        &output,
        "release profile should reject handler request state escaping into a spawned task",
    );
    assert_contains(
        &output.combined(),
        "K0067",
        "request-state leak should use the existing source-mapped diagnostic",
    );
    assert_contains(
        &output.combined(),
        "leak_request",
        "diagnostic should name the handler",
    );
}

#[test]
fn handler_obligation_requires_reply_reject_or_cancel() {
    let project = TestProject::new("handler-obligation");
    let file = project.main_file(
        r#"
struct Request {
    id: u64,
}

#[kobo::handler]
async fn missing_reply(request: Request) {}

#[kobo::scenario(profile = "async")]
async fn obligation_path() {
    let request = Request { id: 1 };
    missing_reply(request).await;
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

    assert_failure(
        &output,
        "handler without reply/reject/cancel should fail the lifecycle obligation",
    );
    assert_contains(
        &output.combined(),
        "reply",
        "diagnostic should explain the missing handler terminal action",
    );
}

#[test]
fn handler_tokens_emit_must_call_metadata() {
    let (project, file) = handler_fixture();
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

    assert_success(&output, "handler success path should satisfy obligations");
    let witness = first_witness(&project);
    assert_eq!(
        witness["handler_lifecycle"]["handlers"][0]["name"], "handle",
        "witness should identify the handler lifecycle surface"
    );
    assert_eq!(
        witness["handler_lifecycle"]["evidence_source"], "codegen-lowering",
        "handler evidence should come from compiler lowering metadata, not source reparsing"
    );
    assert_contains(
        &witness["handler_lifecycle"].to_string(),
        "reply",
        "handler witness should include reply terminal metadata from lowered result paths",
    );
    assert_contains(
        &witness["handler_lifecycle"].to_string(),
        "reject",
        "handler witness should preserve the terminal actions actually present in lowered code",
    );
    assert_contains(
        &witness["handler_lifecycle"].to_string(),
        "drop-runs-registered-cleanup",
        "handler witness should expose cancel cleanup as lifecycle evidence instead of an unconditional terminal action",
    );
    assert_contains(
        &witness["handler_lifecycle"].to_string(),
        "lowered-result-path-guard-scan",
        "handler witness should cite lowered result paths instead of raw source token shape",
    );
}

#[test]
fn handler_disconnect_case_is_scenario_testable() {
    let (project, file) = handler_fixture();
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--target"),
            s("handler_disconnect_path"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "client disconnect path should be scenario-testable",
    );
    let witness = first_witness(&project);
    assert_contains(
        &witness["events"].to_string(),
        "network-dropped",
        "disconnect scenario should include the modeled client disconnect path",
    );
    assert_contains(
        &witness["handler_lifecycle"].to_string(),
        "disconnect",
        "handler evidence should classify the disconnect scenario",
    );
}

#[test]
fn handler_metrics_boundary_is_visible_in_inspect() {
    let generated = inspect_handler();

    assert_contains(
        &generated,
        "metrics_boundary(\"handle\")",
        "handler inspect output should expose the metrics boundary",
    );
    assert_contains(
        &generated,
        "self.metrics.exits += 1",
        "handler metrics boundary should update lifecycle metrics instead of being a no-op",
    );
    assert_not_contains(
        &generated,
        "fn metrics_boundary(&self, _handler: &'static str) {}",
        "handler metrics boundary must not lower to an empty placeholder",
    );
    assert_not_contains(
        &generated,
        "#[kobo::handler]",
        "handler attribute should lower away instead of remaining syntax-only",
    );
}
