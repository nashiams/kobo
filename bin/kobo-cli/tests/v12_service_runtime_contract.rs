mod v09_common;

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
