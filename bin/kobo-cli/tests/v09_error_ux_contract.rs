mod v09_common;

use std::path::{Path, PathBuf};

use kobo_errors::{resolve_color_mode_from_parts, ColorMode};

use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo,
    run_kobo_with_env, s, TestProject,
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

fn assert_guidance_lines_are_wrapped(output: &str) {
    for line in output.lines() {
        let trimmed = line.trim_start();
        let is_guidance = trimmed.starts_with("What Kobo found:")
            || trimmed.starts_with("Why this matters:")
            || trimmed.starts_with("Try this:")
            || trimmed.starts_with("More:")
            || (line.starts_with("  ") && !trimmed.starts_with('|') && !trimmed.starts_with("-->"));
        if is_guidance {
            assert!(
                line.chars().count() <= 100,
                "human guidance line is too long:\n{line}\n\n{output}"
            );
        }
    }
}

fn assert_section_has_body(output: &str, heading: &str) {
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        if line.trim() == heading {
            let Some(next) = lines.next() else {
                panic!("section `{heading}` has no body:\n{output}");
            };
            assert!(
                !next.trim().is_empty(),
                "section `{heading}` has an empty body:\n{output}"
            );
            return;
        }
    }
    panic!("missing section `{heading}`:\n{output}");
}

#[test]
fn color_auto_is_terminal_aware_and_respects_no_color() {
    assert_eq!(
        resolve_color_mode_from_parts(ColorMode::Auto, false, true),
        ColorMode::Always,
        "auto should colorize human output when stderr is a terminal"
    );
    assert_eq!(
        resolve_color_mode_from_parts(ColorMode::Auto, false, false),
        ColorMode::Never,
        "auto should avoid ANSI escapes when stderr is captured"
    );
    assert_eq!(
        resolve_color_mode_from_parts(ColorMode::Always, true, true),
        ColorMode::Never,
        "NO_COLOR must override an explicit color request"
    );
    assert_eq!(
        resolve_color_mode_from_parts(ColorMode::Never, false, true),
        ColorMode::Never,
        "explicit --color=never must remain plain"
    );
}

#[test]
fn human_diagnostic_uses_teaching_sections_without_internal_trailer() {
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
        "What Kobo found:",
        "human diagnostic should name the local problem",
    );
    assert_contains(
        &combined,
        "Why this matters:",
        "human diagnostic should explain why Kobo cares",
    );
    assert_contains(
        &combined,
        "Try this:",
        "human diagnostic should include an actionable fix",
    );
    assert_contains(
        &combined,
        "More:",
        "human diagnostic should point to deeper explain output",
    );
    assert_no_default_human_jank(&combined);
}

#[test]
fn human_output_is_colorized_by_default_when_color_is_forced() {
    let project = TestProject::new("error-ux-color");
    let fixture = ui_fixture("K0025_hint_ignored.kobo");
    let output = run_kobo(
        &[
            s("check"),
            s("--strict"),
            s("--color=always"),
            path_arg(&fixture),
        ],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0025 fixture should fail");
    assert!(
        combined.contains("\u{1b}["),
        "forced color output should contain ANSI escapes:\n{combined}"
    );
    assert_contains(
        &combined,
        "error[K0025]",
        "color must not hide code identity",
    );
    assert_contains(
        &combined,
        "Why this matters:",
        "color must preserve why section",
    );
    assert_contains(&combined, "Try this:", "color must preserve fix section");
}

#[test]
fn no_color_disables_ansi_but_keeps_human_sections() {
    let project = TestProject::new("error-ux-no-color");
    let fixture = ui_fixture("K0025_hint_ignored.kobo");
    let output = run_kobo_with_env(
        &[
            s("check"),
            s("--strict"),
            s("--color=always"),
            path_arg(&fixture),
        ],
        &project.root,
        &[("NO_COLOR", "1")],
    );
    let combined = output.combined();

    assert_failure(&output, "K0025 fixture should fail");
    assert_not_contains(&combined, "\u{1b}[", "NO_COLOR must strip ANSI");
    assert_contains(
        &combined,
        "Why this matters:",
        "NO_COLOR must preserve why section",
    );
    assert_contains(&combined, "Try this:", "NO_COLOR must preserve fix section");
}

#[test]
fn k0025_uses_plain_language_not_solver_language() {
    let project = TestProject::new("error-ux-k0025-plain");
    let fixture = ui_fixture("K0025_hint_ignored.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0025 fixture should fail");
    assert_contains(&combined, "error[K0025]", "expected K0025 diagnostic");
    for forbidden in ["shared-mutable floor", "later facts", "constraint conflict"] {
        assert_not_contains(
            &combined,
            forbidden,
            "K0025 human output should avoid solver/internal wording",
        );
    }
    assert_contains(
        &combined,
        "The hint asks Kobo to move `buf`, but the code mutably uses `buf` more than once.",
        "K0025 should say the direct problem in user language",
    );
    assert_contains(
        &combined,
        "A moved value has only one owner.",
        "K0025 should explain the concrete reason",
    );
}

#[test]
fn k0025_card_matches_plan_teaching_wording() {
    let project = TestProject::new("error-ux-k0025-plan-wording");
    let fixture = ui_fixture("K0025_hint_ignored.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0025 fixture should fail");
    assert_contains(
        &combined,
        "error[K0025]: Kobo cannot use this ownership hint",
        "K0025 title should match the plan's Elm-style wording",
    );
    assert_contains(
        &combined,
        "What Kobo found:\n  The hint asks Kobo to move `buf`, but the code mutably uses `buf` more than once.",
        "K0025 should use the plan's finding sentence",
    );
    assert_contains(
        &combined,
        "Why this matters:\n  A moved value has only one owner. This code needs a shape that can support repeated mutable use.",
        "K0025 should use the plan's why sentence",
    );
    assert_contains(
        &combined,
        "Try this:\n  Remove the hint, change it to match the sharing pattern, or refactor so `buf` has only one owner.",
        "K0025 should use the plan's fix sentence",
    );
}

#[test]
fn k0063_explains_async_strict_without_protocol_jargon() {
    let project = TestProject::new("error-ux-k0063-plain");
    let fixture = ui_fixture("strict_k0063_async_context.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0063 fixture should fail");
    assert_contains(&combined, "error[K0063]", "expected K0063 diagnostic");
    for forbidden in [
        "boundary guards",
        "async-aware guard protocol",
        "makes that rule ambiguous",
    ] {
        assert_not_contains(
            &combined,
            forbidden,
            "K0063 human output should avoid protocol jargon",
        );
    }
    assert_contains(
        &combined,
        "Kobo needs strict borrows to end before the function can pause or be cancelled.",
        "K0063 should explain the concrete async risk",
    );
    assert_contains(
        &combined,
        "pause or be cancelled",
        "K0063 should name the async cancellation risk in plain language",
    );
    assert_contains(
        &combined,
        "small non-async helper",
        "K0063 should offer the ergonomic sync-helper repair path",
    );
}

#[test]
fn k0063_card_matches_plan_teaching_wording() {
    let project = TestProject::new("error-ux-k0063-plan-wording");
    let fixture = ui_fixture("strict_k0063_async_context.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0063 fixture should fail");
    assert_contains(
        &combined,
        "error[K0063]: this strict borrow is inside code that can pause",
        "K0063 title should match the plan's Elm-style wording",
    );
    assert_contains(
        &combined,
        "async functions can pause at `.await`",
        "K0063 secondary label should teach the async rule",
    );
    assert_contains(
        &combined,
        "this strict borrow starts here",
        "K0063 primary label should name the exact start site",
    );
    assert_contains(
        &combined,
        "What Kobo found:\n  This strict block runs inside an async function.",
        "K0063 should use the plan's finding sentence",
    );
    assert_contains(
        &combined,
        "Why this matters:\n  Kobo needs strict borrows to end before the function can pause or be cancelled.",
        "K0063 should use the plan's why sentence",
    );
    assert_contains(
        &combined,
        "Try this:\n  1. Use `@strict async fn` if the whole function should follow Kobo's async strict rules.",
        "K0063 should render the first numbered fix choice",
    );
    assert_contains(
        &combined,
        "  2. Or move this strict work into a small non-async helper.",
        "K0063 should render the second numbered fix choice",
    );
    assert_contains(
        &combined,
        "More:\n  Run `kobo explain K0063`",
        "K0063 should point to the explain page in the plan style",
    );
}

#[test]
fn human_why_and_fix_lines_are_wrapped() {
    let project = TestProject::new("error-ux-wrapped-guidance");
    let fixture = ui_fixture("strict_k0063_async_context.kobo");
    let output = run_kobo(
        &[s("check"), s("--strict"), path_arg(&fixture)],
        &project.root,
    );
    let combined = output.combined();

    assert_failure(&output, "K0063 fixture should fail");
    assert_guidance_lines_are_wrapped(&combined);
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
    assert_section_has_body(&combined, "Try this:");
    assert_section_has_body(&combined, "More:");
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
