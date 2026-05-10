mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_mentions_line, assert_not_contains, assert_success,
    fixture_template, one_based_line_of, path_arg, run_kobo, s, unique_symbol, TestProject,
};

#[test]
fn trap_string_only_profiles_do_not_count() {
    let project = TestProject::new("trap-string-profile");
    let file = project.copy_fixture("traps/string_profile.kobo", "src/main.kobo");

    let output = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("release"),
            s("--print-policy=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "release profile should print policy");
    let text = output.combined();
    assert_contains(
        &text,
        "ownership",
        "policy output must include ownership guarantee",
    );
    assert_contains(
        &text,
        "liveness",
        "policy output must include liveness guarantee",
    );
    assert_contains(
        &text,
        "replay",
        "policy output must include replay guarantee",
    );
    assert_contains(
        &text,
        "boundaries",
        "policy output must include boundary guarantee",
    );
}

#[test]
fn trap_backend_imports_never_leak_into_user_source() {
    let project = TestProject::new("trap-backend-imports");
    let file = project.copy_fixture("traps/backend_imports.kobo", "src/gateway.kobo");

    let output = run_kobo(
        &[
            s("sim"),
            s("init"),
            s("--target"),
            format!("{}:handle_request", path_arg(&file)),
            s("--profile"),
            s("async"),
            s("--minimal"),
        ],
        &project.root,
    );

    assert_success(&output, "sim init should succeed");
    let source = project.read("src/gateway.kobo");
    assert_not_contains(&source, "loom::", "normal user source must not import Loom");
    assert_not_contains(
        &source,
        "shuttle::",
        "normal user source must not import Shuttle",
    );
}

#[test]
fn trap_quick_budget_overflow_emits_k0105_instead_of_hanging() {
    let project = TestProject::new("trap-budget");
    let file = project.copy_fixture("sim/budget_overflow.kobo", "src/main.kobo");

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--event-budget"),
            s("8"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "budget overflow should fail quickly");
    let text = output.combined();
    assert_contains(&text, "K0105", "budget overflow must emit K0105");
    assert_contains(&text, "8", "diagnostic must include configured budget");
}

#[test]
fn trap_diagnostic_spans_move_with_source_offsets() {
    let project = TestProject::new("trap-span-shift");
    let first_type = unique_symbol("DeliveryA");
    let second_type = unique_symbol("DeliveryB");
    let first_source =
        fixture_template("sim/span_first.template.kobo", &[("__TYPE__", &first_type)]);
    let second_source = fixture_template(
        "sim/span_second.template.kobo",
        &[("__TYPE__", &second_type)],
    );
    let first_line = one_based_line_of(&first_source, "let _lost = delivery");
    let second_line = one_based_line_of(&second_source, "let _lost = delivery");
    assert_ne!(
        first_line, second_line,
        "fixture must exercise shifted spans"
    );
    let first_file = project.write("src/first_span.kobo", &first_source);
    let second_file = project.write("src/second_span.kobo", &second_source);

    let first = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&first_file),
        ],
        &project.root,
    );
    let second = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&second_file),
        ],
        &project.root,
    );

    assert_failure(&first, "first shifted liveness fixture should fail");
    assert_failure(&second, "second shifted liveness fixture should fail");
    assert_contains(
        &first.combined(),
        "K0100",
        "first fixture must emit liveness code",
    );
    assert_contains(
        &second.combined(),
        "K0100",
        "second fixture must emit liveness code",
    );
    assert_mentions_line(
        &first,
        first_line,
        "first diagnostic must point at actual source line",
    );
    assert_mentions_line(
        &second,
        second_line,
        "second diagnostic must move when source line moves",
    );
}

#[test]
fn trap_lsp_uses_registry_payload_for_k010x_actions() {
    let project = TestProject::new("trap-lsp");
    let file = project.copy_fixture("lsp/replay_http.kobo", "src/main.kobo");

    let output = run_kobo(
        &[
            s("lsp-diagnostics"),
            s("--format=json"),
            s("--no-project-ok"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "lsp diagnostics should succeed");
    let text = output.combined();
    assert_contains(&text, "K0107", "LSP must publish K0107");
    assert_contains(&text, "data", "LSP payload must include structured data");
    assert_contains(&text, "kobo explain", "LSP action must expose explain path");
    assert_contains(&text, "boundary", "LSP action must group boundary choices");
}
