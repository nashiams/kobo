mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_test_support::{
    assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn source(target: &str, attrs: &str, body: &str) -> String {
    format!(
        r#"
{attrs}
#[kobo::scenario(profile = "sync")]
fn {target}() {{
{body}
}}
"#
    )
}

fn emit(project: &TestProject, source: &str, target: &str) -> PathBuf {
    let file = project.main_file(source);
    let artifact_path = project.root.join(format!("{target}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s(target),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&output, "spike candidate artifact should emit");
    artifact_path
}

fn emit_failure(project: &TestProject, source: &str, target: &str, expected: &str) {
    let file = project.main_file(source);
    let artifact_path = project.root.join(format!("{target}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s(target),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_failure(&output, "invalid ergonomic spike admission should fail");
    assert!(
        output.combined().contains(expected),
        "failure should mention `{expected}`\n{}",
        output.combined()
    );
}

fn artifact(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

fn fact<'a>(candidate: &'a Value, key: &str) -> &'a Value {
    candidate["evidence"]
        .as_array()
        .expect("candidate evidence facts should be an array")
        .iter()
        .find(|fact| fact["key"].as_str() == Some(key))
        .unwrap_or_else(|| panic!("missing candidate evidence fact `{key}`: {candidate}"))
}

#[test]
fn inspectable_sugar_transforms_name_manual_rust_equivalents() {
    let project = TestProject::new("proof-sugar-transforms");
    let path = emit(
        &project,
        &source(
            "sugar_transform_case",
            r#"#[kobo::sugar(builder, visitor, state_machine, event_enum)]
#[kobo::candidate_track(
    id = "S-46",
    track = "Proc-macro replacement DSL",
    status = "research",
    inspect = "inspect shows builder visitor state-machine event-enum transforms",
    manual_rust = "write builders, visitors, state machines, and event enums by hand",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_sugar_snapshot",
    replay_related = "false"
)]"#,
            "    let _value = 1u32;",
        ),
        "sugar_transform_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(
        fact(candidate, "transform_set")["value"],
        "builder|visitor|state_machine|event_enum"
    );
    assert_eq!(fact(candidate, "desugaring")["value"], "inspectable");
}

#[test]
fn orphan_newtype_scaffolding_respects_coherence_rules() {
    let project = TestProject::new("proof-newtype-scaffold");
    let path = emit(
        &project,
        &source(
            "newtype_scaffold_case",
            r#"#[kobo::newtype_scaffold(wrapper = "PaymentId", forwarding = "true")]
#[kobo::candidate_track(
    id = "S-47",
    track = "Orphan-rule newtype scaffolding",
    status = "research",
    inspect = "inspect shows wrapper and forwarding impls",
    manual_rust = "write the newtype wrapper and forwarding impls manually",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_newtype_snapshot",
    replay_related = "false"
)]"#,
            "    let _value = 1u32;",
        ),
        "newtype_scaffold_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "coherence")["value"], "newtype_forwarding");
    assert_eq!(fact(candidate, "hidden_impls")["value"], "false");
}

#[test]
fn context_injection_is_explicit_and_has_no_hidden_globals() {
    let project = TestProject::new("proof-context-threading");
    let path = emit(
        &project,
        &source(
            "context_threading_case",
            r#"#[kobo::candidate_track(
    id = "S-48",
    track = "Explicit context injection",
    status = "research",
    inspect = "inspect shows explicit context threading",
    manual_rust = "pass context parameters explicitly",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_context_snapshot",
    replay_related = "false"
)]"#,
            "    struct Context;\n    let ctx = Context;\n    let _value = &ctx;",
        ),
        "context_threading_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "context_threading")["value"], "explicit");
    assert_eq!(fact(candidate, "hidden_globals")["value"], "false");
}

#[test]
fn hidden_global_blocks_graduated_context_injection() {
    let project = TestProject::new("proof-context-hidden-global-fails");

    emit_failure(
        &project,
        &format!(
            r#"
static mut GLOBAL_CONTEXT: u8 = 0;

{}
"#,
            source(
                "hidden_global_context_case",
                r#"#[kobo::candidate_track(
    id = "S-48",
    track = "Explicit context injection",
    status = "graduate",
    inspect = "inspect shows explicit context threading",
    manual_rust = "pass context parameters explicitly",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_context_snapshot",
    replay_related = "false"
)]"#,
                "    let _value = unsafe { GLOBAL_CONTEXT };",
            )
        ),
        "hidden_global_context_case",
        "hidden globals",
    );
}
