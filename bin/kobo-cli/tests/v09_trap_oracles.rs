mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo, s,
    TestProject,
};

#[test]
fn trap_string_only_profiles_do_not_count() {
    let project = TestProject::new("trap-string-profile");
    let file = project.main_file("fn main() {}\n");

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
    assert_contains(&text, "ownership", "policy output must include ownership guarantee");
    assert_contains(&text, "liveness", "policy output must include liveness guarantee");
    assert_contains(&text, "replay", "policy output must include replay guarantee");
    assert_contains(&text, "boundaries", "policy output must include boundary guarantee");
}

#[test]
fn trap_backend_imports_never_leak_into_user_source() {
    let project = TestProject::new("trap-backend-imports");
    let file = project.write(
        "src/gateway.kobo",
        r#"
async fn handle_request() {}
"#,
    );

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
    assert_not_contains(&source, "shuttle::", "normal user source must not import Shuttle");
}

#[test]
fn trap_quick_budget_overflow_emits_k0105_instead_of_hanging() {
    let project = TestProject::new("trap-budget");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn never_finishes() {
    loop {}
}
"#,
    );

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
fn trap_lsp_uses_registry_payload_for_k010x_actions() {
    let project = TestProject::new("trap-lsp");
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

