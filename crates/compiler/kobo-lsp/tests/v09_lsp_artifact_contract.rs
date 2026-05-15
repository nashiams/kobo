#[test]
fn replay_action_rejects_stale_witness_and_uses_artifact_payload() {
    let diagnostic = kobo_lsp::test_support::diagnostic_with_artifact(
        "K0100",
        ".kobo/witnesses/current.kwit",
        "source-hash-current",
    );
    let stale =
        kobo_lsp::test_support::witness_artifact(".kobo/witnesses/stale.kwit", "source-hash-stale");
    let current = kobo_lsp::test_support::witness_artifact(
        ".kobo/witnesses/current.kwit",
        "source-hash-current",
    );
    let actions = kobo_lsp::actions_for_diagnostic_with_artifacts(&diagnostic, &[stale, current]);
    let text = serde_json::to_string(&actions).unwrap();
    assert!(text.contains(".kobo/witnesses/current.kwit"));
    assert!(!text.contains(".kobo/witnesses/stale.kwit"));
    assert!(!text.contains("<witness>.kwit"));
}
