mod v09_common;

use std::fs;

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
fn service_cancellation_token_is_wired_into_shutdown() {
    let generated = inspect_service(64);

    for expected in [
        "KoboServiceCancellationToken::new()",
        "self.cancellation.cancel();",
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
            "{}\n\n#[kobo::scenario(profile = \"async\")]\nfn submit_path() {{\n    ward.task();\n}}\n",
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
        witness["service_runtime"]["services"][0]["buffer"], 64,
        "witness should preserve the generated service channel capacity"
    );
    assert_contains(
        &witness["service_runtime"]["services"][0]["methods"].to_string(),
        "submit",
        "service witness should include method-level scenario hooks",
    );
    assert_contains(
        &witness["events"].to_string(),
        "deterministic-task",
        "service scenario should still drive the modeled task path",
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
