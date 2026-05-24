mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_mentions_line, assert_not_contains, assert_success,
    fixture_text, one_based_line_of, path_arg, run_kobo, run_kobo_lsp_stdio, s, TestProject,
};

#[test]
fn lsp_publishes_k010x_payloads_witness_links_and_actions() {
    let project = TestProject::new("lsp-k010x");
    let file = project.copy_fixture("lsp/replay_http.kobo", "src/main.kobo");
    let sim = run_kobo(
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
    assert_failure(&sim, "LSP artifact contract needs a real witness first");

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
    let sim = run_kobo(
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
    assert_failure(&sim, "failing scenario should emit a witness before LSP");

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
fn lsp_links_existing_witness_path_instead_of_placeholder() {
    let project = TestProject::new("lsp-existing-kwit-link");
    let file = project.copy_fixture("lsp/leak_delivery.kobo", "src/main.kobo");
    let sim = run_kobo(
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
    assert_failure(&sim, "failing scenario should emit a witness");
    let witnesses = project.find_files_with_ext("kwit");
    assert!(
        !witnesses.is_empty(),
        "witness must exist before LSP lookup"
    );
    let witness_name = witnesses[0]
        .file_name()
        .and_then(|name| name.to_str())
        .expect("witness filename should be UTF-8");

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
    assert_contains(&text, witness_name, "LSP should link the emitted witness");
    assert_not_contains(
        &text,
        "<target>.kwit",
        "LSP witness link must not be a placeholder",
    );
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
    let first_sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&first_file),
        ],
        &project.root,
    );
    let second_sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&second_file),
        ],
        &project.root,
    );
    assert_failure(&first_sim, "first boundary scenario should emit a witness");
    assert_failure(
        &second_sim,
        "second boundary scenario should emit a witness",
    );

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

#[test]
fn stdio_lsp_links_only_matching_witness_for_open_document() {
    let project = TestProject::new("lsp-stdio-fresh-witness");
    let first_source = fixture_text("lsp/range_first.kobo");
    let second_source = fixture_text("lsp/range_second.kobo");
    let first_file = project.write("src/lsp_first.kobo", &first_source);
    let second_file = project.write("src/lsp_second.kobo", &second_source);
    let first_sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&first_file),
        ],
        &project.root,
    );
    let second_sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&second_file),
        ],
        &project.root,
    );
    assert_failure(&first_sim, "first boundary scenario should emit a witness");
    assert_failure(
        &second_sim,
        "second boundary scenario should emit a newer witness",
    );
    let mut witnesses = project.find_files_with_ext("kwit");
    witnesses.sort();
    assert!(
        witnesses.len() >= 2,
        "test requires two witnesses to prove stale rejection"
    );
    let first_witness = witnesses
        .iter()
        .find(|path| path.to_string_lossy().contains("replay_http_first"))
        .expect("first source witness should exist");
    let second_witness = witnesses
        .iter()
        .find(|path| path.to_string_lossy().contains("replay_http_second"))
        .expect("second source witness should exist");
    let first_witness_name = first_witness
        .file_name()
        .and_then(|name| name.to_str())
        .expect("first witness filename should be UTF-8");
    let second_witness_name = second_witness
        .file_name()
        .and_then(|name| name.to_str())
        .expect("second witness filename should be UTF-8");
    let first_uri = file_uri(&first_file);
    let input = [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        })
        .to_string(),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": first_uri,
                    "languageId": "kobo",
                    "version": 1,
                    "text": first_source,
                }
            }
        })
        .to_string(),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/documentLink",
            "params": {
                "textDocument": {"uri": first_uri}
            }
        })
        .to_string(),
    ]
    .join("\n");

    let output = run_kobo_lsp_stdio(&input, &project.root);

    assert_success(&output, "stdio LSP should process open document");
    let text = output.combined();
    assert_contains(
        &text,
        first_witness_name,
        "stdio LSP should link the witness matching the opened source",
    );
    assert_not_contains(
        &text,
        second_witness_name,
        "stdio LSP must reject newer witnesses for other source hashes",
    );
}

fn file_uri(path: &std::path::Path) -> String {
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    if normalized
        .as_bytes()
        .get(1)
        .is_some_and(|byte| *byte == b':')
    {
        normalized.insert(0, '/');
    }
    format!("file://{}", normalized.replace(' ', "%20"))
}
