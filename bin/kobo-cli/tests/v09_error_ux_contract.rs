mod v09_common;

use std::path::{Path, PathBuf};

use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo, s,
    TestProject,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn ui_fixture(relative: &str) -> PathBuf {
    repo_root().join("tests").join("ui").join(relative)
}

fn assert_no_default_human_jank(output: &str) {
    for needle in [
        "kobo decision:",
        "greedy priority",
        "engine ceiling",
        "Error: analysis failed",
        "\u{00e2}",
        "\u{00c3}\u{00a2}",
        "\u{fffd}",
    ] {
        assert!(
            !output.contains(needle),
            "human diagnostic leaked `{needle}`:\n{output}"
        );
    }
}

#[test]
fn human_diagnostic_uses_why_and_fix_without_internal_trailer() {
    let project = TestProject::new("error-ux-k0025");
    let fixture = ui_fixture("K0025_hint_ignored.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0025 fixture should fail");
    assert_contains(&combined, "error[K0025]", "expected K0025 diagnostic");
    assert_contains(
        &combined,
        "why:",
        "human diagnostic should explain why Kobo cares",
    );
    assert!(
        combined.contains("fix:") || combined.contains("try this:"),
        "human diagnostic should include an actionable fix:\n{combined}"
    );
    assert_no_default_human_jank(&combined);
}

#[test]
fn strict_async_diagnostic_does_not_show_mojibake_or_empty_fix() {
    let project = TestProject::new("error-ux-k0063");
    let fixture = ui_fixture("strict_k0063_async_context.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0063 fixture should fail");
    assert_contains(&combined, "error[K0063]", "expected K0063 diagnostic");
    assert_no_default_human_jank(&combined);
    assert!(
        !combined
            .lines()
            .any(|line| line.trim() == "fix:" || line.trim() == "try this:"),
        "fix line must not be empty:\n{combined}"
    );
}

#[test]
fn explain_is_teaching_page_by_default_not_registry_dump() {
    let project = TestProject::new("error-ux-explain");
    let output = run_kobo(&[s("explain"), s("K0063")], &project.root);
    let combined = output.combined();

    assert_success(&output, "kobo explain should succeed");
    assert!(
        combined.contains("What happened") || combined.contains("What Kobo found"),
        "default explain should teach the user:\n{combined}"
    );
    assert!(
        combined.contains("How to fix") || combined.contains("What to do"),
        "default explain should include remediation:\n{combined}"
    );
    assert_not_contains(
        &combined,
        "machine edits:",
        "registry metadata should be verbose-only",
    );
    assert_not_contains(
        &combined,
        "status: active",
        "registry status should be verbose-only",
    );
}

#[test]
fn verbose_explain_keeps_registry_metadata() {
    let project = TestProject::new("error-ux-explain-verbose");
    let output = run_kobo(&[s("explain"), s("K0063"), s("--verbose")], &project.root);
    let combined = output.combined();

    assert_success(&output, "verbose explain should succeed");
    assert_contains(&combined, "slug:", "verbose explain should include slug");
    assert_contains(
        &combined,
        "machine edits:",
        "verbose explain should include machine edit policy",
    );
    assert_contains(
        &combined,
        "mode policy:",
        "verbose explain should include mode policy",
    );
}

#[test]
fn json_output_keeps_machine_metadata_even_when_human_output_is_cleaned() {
    let project = TestProject::new("error-ux-json");
    let fixture = ui_fixture("K0025_hint_ignored.kobo");
    let output = run_kobo(
        &[
            s("check"),
            s("--strict"),
            s("--error-format=json"),
            path_arg(&fixture),
        ],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0025 JSON fixture should fail");
    assert_contains(
        &combined,
        "\"code\":\"K0025\"",
        "JSON diagnostics must keep stable code identity",
    );
    assert_contains(
        &combined,
        "\"severity\"",
        "JSON diagnostics must keep severity metadata",
    );
    assert_contains(
        &combined,
        "\"primary\"",
        "JSON diagnostics must keep primary span metadata",
    );
}

#[test]
fn human_output_avoids_raw_byte_spans_when_source_line_exists() {
    let project = TestProject::new("error-ux-raw-bytes");
    let fixture = ui_fixture("strict_k0042_closure_capture.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0041/K0042 fixture should fail");
    assert!(
        combined.contains("error[K0041]") || combined.contains("error[K0042]"),
        "expected strict boundary diagnostic:\n{combined}"
    );
    assert_not_contains(
        &combined,
        "[bytes ",
        "human diagnostics should not show raw byte spans when a source line exists",
    );
}
