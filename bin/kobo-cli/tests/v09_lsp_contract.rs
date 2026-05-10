mod v09_common;

use v09_common::{
    assert_contains, assert_mentions_line, assert_not_contains, assert_success, fixture_text,
    one_based_line_of, path_arg, run_kobo, s, TestProject,
};

#[test]
fn lsp_publishes_k010x_payloads_witness_links_and_actions() {
    let project = TestProject::new("lsp-k010x");
    let file = project.copy_fixture("lsp/replay_http.kobo", "src/main.kobo");

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

    assert_success(
        &output,
        "LSP diagnostics should be available from public CLI path",
    );
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
        assert_contains(
            &text,
            needle,
            "LSP K010x payload must be registry-backed and actionable",
        );
    }
}

#[test]
fn lsp_uses_kwit_link_when_failure_has_witness() {
    let project = TestProject::new("lsp-kwit-link");
    let file = project.copy_fixture("lsp/leak_delivery.kobo", "src/main.kobo");

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

#[test]
fn lsp_does_not_emit_liveness_failure_when_must_call_is_resolved() {
    let project = TestProject::new("lsp-resolved-must-call");
    let file = project.write(
        "src/main.kobo",
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn resolved_delivery() {
    let delivery = Delivery {};
    delivery.ack();
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

    assert_success(&output, "resolved LSP diagnostics should succeed");
    assert_not_contains(
        &output.combined(),
        "K0100",
        "resolved must_call scenario must not publish liveness failure",
    );
}

#[test]
fn lsp_ranges_move_when_boundary_call_moves() {
    let project = TestProject::new("lsp-range-shift");
    let first_source = fixture_text("lsp/range_first.kobo");
    let second_source = fixture_text("lsp/range_second.kobo");
    let first_line = one_based_line_of(&first_source, "reqwest::Client::new");
    let second_line = one_based_line_of(&second_source, "reqwest::Client::new");
    assert_ne!(
        first_line, second_line,
        "fixture must shift the boundary call"
    );
    let first_file = project.write("src/lsp_first.kobo", &first_source);
    let second_file = project.write("src/lsp_second.kobo", &second_source);

    let first = run_kobo(
        &[
            s("lsp-diagnostics"),
            s("--format=json"),
            s("--no-project-ok"),
            s("--include-actions"),
            path_arg(&first_file),
        ],
        &project.root,
    );
    let second = run_kobo(
        &[
            s("lsp-diagnostics"),
            s("--format=json"),
            s("--no-project-ok"),
            s("--include-actions"),
            path_arg(&second_file),
        ],
        &project.root,
    );

    assert_success(&first, "first LSP diagnostics should succeed");
    assert_success(&second, "second LSP diagnostics should succeed");
    assert_contains(
        &first.combined(),
        "K0107",
        "first LSP output must diagnose boundary",
    );
    assert_contains(
        &second.combined(),
        "K0107",
        "second LSP output must diagnose boundary",
    );
    assert_mentions_line(
        &first,
        first_line,
        "first LSP range must use actual source line",
    );
    assert_mentions_line(
        &second,
        second_line,
        "second LSP range must move with source",
    );
}
