mod v09_common;

use std::fs;
use std::process::Command;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo, s, TestProject,
};

fn service_fixture(buffer: usize) -> (TestProject, std::path::PathBuf) {
    let project = TestProject::new(&format!("service-runtime-buffer-{buffer}"));
    let file = project.main_file(&format!(
        r#"
struct Gateway {{}}

struct Request {{
    id: u64,
    body: String,
}}

struct Response {{
    status: u16,
}}

#[kobo::service(buffer={buffer})]
impl Gateway {{
    async fn submit(&self, request: Request, attempt: u64) -> Response {{
        Response {{ status: 202 }}
    }}

    async fn refresh(&self, key: String) {{
        let _seen = key;
    }}
}}
"#
    ));
    (project, file)
}

fn inspect_service(buffer: usize) -> String {
    let (project, file) = service_fixture(buffer);
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(&output, "service fixture should inspect");
    output.combined()
}

fn mutable_service_fixture() -> (TestProject, std::path::PathBuf) {
    let project = TestProject::new("service-runtime-mutable-state");
    let file = project.main_file(
        r#"
struct Counter {
    value: u64,
}

#[kobo::service(buffer=4)]
impl Counter {
    async fn increment(&mut self, amount: u64) -> u64 {
        self.value = self.value + amount;
        self.value
    }
}

#[tokio::main]
async fn main() {
    let (client, worker) = CounterService::start(Counter { value: 0 });
    let _first = client.increment(1).await.expect("first increment");
    let _second = client.increment(2).await.expect("second increment");
    client.shutdown_and_wait(worker).await.expect("shutdown should run");
}
"#,
    );
    (project, file)
}

fn first_witness(project: &TestProject) -> Value {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("service simulation should write a witness");
    serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
        .expect("witness should parse")
}

#[test]
fn service_generates_typed_message_enum_and_bounded_channel() {
    let generated = inspect_service(64);

    for expected in [
        "enum GatewayMessage",
        "Submit {",
        "request: Request",
        "attempt: u64",
        "__reply: tokio::sync::oneshot::Sender<Response>",
        "Refresh {",
        "key: String",
        "__reply: tokio::sync::oneshot::Sender<()>",
        "Shutdown",
        "tokio::sync::mpsc::channel::<GatewayMessage>(64)",
        "KoboServiceCancellationToken",
    ] {
        assert_contains(
            &generated,
            expected,
            "service lowering should generate typed service runtime Rust",
        );
    }
    assert_not_contains(
        &generated,
        "shuttle::",
        "service lowering must not import backend scheduler crates into user output",
    );
}

#[test]
fn service_buffer_size_changes_generated_channel_capacity() {
    let generated_64 = inspect_service(64);
    let generated_7 = inspect_service(7);

    assert_contains(
        &generated_64,
        "tokio::sync::mpsc::channel::<GatewayMessage>(64)",
        "buffer=64 should reach generated channel capacity",
    );
    assert_contains(
        &generated_7,
        "tokio::sync::mpsc::channel::<GatewayMessage>(7)",
        "mutating the service buffer should change generated Rust",
    );
}

#[test]
fn service_backpressure_default_is_block_on_full() {
    let generated = inspect_service(64);

    assert_contains(
        &generated,
        "self.sender.send(message).await",
        "default service backpressure should await bounded-channel capacity",
    );
    assert_not_contains(
        &generated,
        "try_send(message)",
        "default service backpressure must not silently drop or fail on full channels",
    );
}

#[test]
fn service_shutdown_path_is_source_mapped_and_inspect_visible() {
    let generated = inspect_service(64);

    for expected in [
        "fn shutdown",
        "GatewayMessage::Shutdown",
        "kobo: service Gateway source_line=13",
    ] {
        assert_contains(
            &generated,
            expected,
            "shutdown and generated service artifacts should stay inspect-visible and source-mapped",
        );
    }
}

#[test]
fn service_runtime_source_map_carries_generated_support_evidence() {
    let (project, file) = service_fixture(64);
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(&output, "service fixture should inspect");
    let source_map_path = file.with_extension("kobo.map");
    let source_map: Value = serde_json::from_str(
        &fs::read_to_string(&source_map_path).expect("service source map should read"),
    )
    .expect("service source map should parse");

    assert_eq!(
        source_map["runtime_evidence"]["services"][0]["name"], "Gateway",
        "source map should carry generated service support evidence, not only doc comments"
    );
    assert_eq!(
        source_map["runtime_evidence"]["services"][0]["source_line"], 13,
        "service support evidence should preserve the source line in the source map"
    );
}

#[test]
fn service_generated_cargo_supports_mutable_state_and_public_api() {
    let (project, file) = mutable_service_fixture();
    let cargo_dir = project.root.join("target").join("mutable-service-cargo");
    let output = run_kobo(
        &[
            s("inspect"),
            s("--cargo"),
            path_arg(&cargo_dir),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &output,
        "mutable service fixture should generate Cargo output",
    );
    let generated = fs::read_to_string(cargo_dir.join("src").join("main.rs"))
        .expect("generated mutable service source should read");
    for expected in [
        "pub enum CounterMessage",
        "pub struct CounterService",
        "pub enum CounterServiceError",
        "mut service: Counter",
        "service.increment(amount).await",
        "CounterMessage::Shutdown",
        "KoboServiceScenarioHook::shutdown",
    ] {
        assert_contains(
            &generated,
            expected,
            "mutable generated service should be production-shaped Rust",
        );
    }

    let check = Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(cargo_dir.join("Cargo.toml"))
        .current_dir(&project.root)
        .output()
        .expect("cargo check should run for generated mutable service fixture");
    assert!(
        check.status.success(),
        "generated mutable service Cargo fixture should compile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn service_generates_dispatch_loop_and_client_methods() {
    let generated = inspect_service(64);

    for expected in [
        "async fn serve(",
        "while let Some(message) = receiver.recv().await",
        "match message",
        "GatewayMessage::Submit",
        "service.submit(request, attempt).await",
        "GatewayMessage::Refresh",
        "service.refresh(key).await",
        "async fn submit(",
        "async fn refresh(",
        "tokio::sync::oneshot::channel",
        "__reply_rx.await",
        "KoboServiceScenarioHook::before",
        "KoboServiceScenarioHook::after",
        "struct KoboServiceScenarioHookEvent",
        "KOBO_SERVICE_SCENARIO_HOOKS",
        "fn events() -> Vec<KoboServiceScenarioHookEvent>",
    ] {
        assert_contains(
            &generated,
            expected,
            "service lowering should generate an executable dispatch loop and typed client API",
        );
    }
}

#[test]
fn service_cancellation_token_is_wired_into_shutdown() {
    let generated = inspect_service(64);

    for expected in [
        "KoboServiceCancellationToken::new()",
        "self.sender.send(GatewayMessage::Shutdown).await",
        "fn is_shutdown_requested",
        "self.cancellation.is_cancelled()",
    ] {
        assert_contains(
            &generated,
            expected,
            "service shutdown should expose cancellation-token wiring",
        );
    }
}

#[test]
fn service_sim_quick_runs_one_critical_path_without_manual_runtime_plumbing() {
    let (project, file) = service_fixture(64);
    project.write(
        "src/main.kobo",
        &format!(
            "{}\n\n#[kobo::scenario(profile = \"async\")]\nasync fn submit_path() {{\n    let (client, worker) = GatewayService::start(Gateway {{}});\n    let _response = client.submit(Request {{ id: 7, body: \"payload\".to_owned() }}, 1).await.expect(\"submit reply\");\n    client.refresh(\"cache\".to_owned()).await.expect(\"refresh reply\");\n    client.shutdown_and_wait(worker).await.expect(\"shutdown\");\n    ward.task();\n}}\n",
            project.read("src/main.kobo")
        ),
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

    assert_success(
        &output,
        "service scenario should run without manual runtime plumbing",
    );
    let witness = first_witness(&project);
    assert_eq!(
        witness["service_runtime"]["services"][0]["name"], "Gateway",
        "witness should identify the service that participates in the scenario"
    );
    assert_eq!(
        witness["service_runtime"]["evidence_source"], "codegen-lowering",
        "service evidence should come from compiler lowering metadata, not source reparsing"
    );
    assert_eq!(
        witness["service_runtime"]["services"][0]["buffer"], 64,
        "witness should preserve the generated service channel capacity"
    );
    assert_contains(
        &witness["service_runtime"]["services"][0]["methods"].to_string(),
        "submit",
        "service witness should include method-level scenario hooks",
    );
    assert_contains(
        &witness["service_runtime"]["services"][0]["hook_events"].to_string(),
        "before",
        "service witness should expose generated hook events used by the client/dispatch path",
    );
    assert_contains(
        &witness["service_runtime"]["services"][0]["runtime_hook_events"].to_string(),
        "submit",
        "service witness should consume runtime hook events emitted by the generated dispatch path",
    );
    assert_contains(
        &witness["service_runtime"]["services"][0]["hook_events"].to_string(),
        "shutdown",
        "service witness should expose shutdown hook evidence",
    );
    assert_contains(
        &witness["events"].to_string(),
        "deterministic-task",
        "service scenario should still drive the modeled task path",
    );
    assert_contains(
        &witness["events"].to_string(),
        "service-scheduler-hook",
        "service scenario hooks should enter the deterministic harness event stream",
    );
}

#[test]
fn service_does_not_generate_backend_imports_in_user_source() {
    let generated = inspect_service(64);

    for backend_import in ["shuttle::", "loom::", "turmoil::", "madsim::"] {
        assert_not_contains(
            &generated,
            backend_import,
            "service lowering should remain clean Rust without backend scheduler imports",
        );
    }
}
