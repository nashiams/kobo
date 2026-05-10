mod v09_common;

use v09_common::{assert_contains, assert_success, path_arg, run_kobo, s, TestProject};

#[test]
fn lsp_publishes_k010x_payloads_witness_links_and_actions() {
    let project = TestProject::new("lsp-k010x");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
async fn replay_http() {
    let _ = reqwest::Client::new();
}
"#,
    );

    let output = run_kobo(
        &[
            s("lsp-diagnostics"),
            s("--format=json"),
            s("--no-project-ok"),
            s("--include-actions"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "LSP diagnostics should be available from public CLI path");
    let text = output.combined();
    for needle in [
        "K0107",
        "slug",
        "title",
        "severity",
        "range",
        "kobo explain",
        "kobo test --sim quick",
        "kobo replay",
        "boundary",
        "codeAction",
    ] {
        assert_contains(&text, needle, "LSP K010x payload must be registry-backed and actionable");
    }
}

#[test]
fn lsp_uses_kwit_link_when_failure_has_witness() {
    let project = TestProject::new("lsp-kwit-link");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn leak_delivery() {
    let delivery = Delivery {};
    let _lost = delivery;
}
"#,
    );

    let output = run_kobo(
        &[
            s("lsp-diagnostics"),
            s("--format=json"),
            s("--no-project-ok"),
            s("--include-actions"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "LSP diagnostics should succeed");
    let text = output.combined();
    assert_contains(&text, "K0100", "LSP should publish liveness failure");
    assert_contains(&text, ".kwit", "LSP should expose witness link");
}

