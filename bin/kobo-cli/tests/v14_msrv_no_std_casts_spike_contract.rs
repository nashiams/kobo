mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use v09_common::{assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject};

const V14_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V14_TIMEOUT)
}

fn source(target: &str, attr: &str) -> String {
    format!(
        r#"
{attr}
#[kobo::scenario(profile = "sync")]
fn {target}() {{
    let _value = 1u32;
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
fn target_rust_and_nightly_avoidance_are_admission_facts() {
    let project = TestProject::new("v14-msrv-target");
    let path = emit(
        &project,
        &source(
            "msrv_target_case",
            r#"#[kobo::candidate_track(
    id = "S-32",
    track = "MSRV-aware emission",
    status = "research",
    inspect = "inspect shows target Rust lowering",
    manual_rust = "write stable Rust 1.70 syntax by hand",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "v14_msrv_snapshot",
    replay_related = "false",
    target_rust = "1.70",
    avoids_nightly = "true"
)]"#,
        ),
        "msrv_target_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "target_rust")["value"], "1.70");
    assert_eq!(fact(candidate, "avoids_nightly")["value"], "true");
}

#[test]
fn no_std_hidden_heap_and_memory_budget_are_admission_facts() {
    let project = TestProject::new("v14-nostd-budget");
    let path = emit(
        &project,
        &source(
            "nostd_budget_case",
            r#"#[kobo::candidate_track(
    id = "S-49",
    track = "no_std allocation budget",
    status = "research",
    inspect = "inspect shows allocation report",
    manual_rust = "write no_std code with explicit buffers",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "v14_nostd_snapshot",
    replay_related = "false",
    no_hidden_heap = "true",
    allocation_report = "structural",
    memory_budget = "deterministic"
)]"#,
        ),
        "nostd_budget_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "no_hidden_heap")["value"], "true");
    assert_eq!(fact(candidate, "allocation_report")["value"], "structural");
    assert_eq!(fact(candidate, "memory_budget")["value"], "deterministic");
}

#[test]
fn cast_policy_and_strict_cast_debt_are_admission_facts() {
    let project = TestProject::new("v14-cast-policy");
    let path = emit(
        &project,
        &source(
            "cast_policy_case",
            r#"#[kobo::candidate_track(
    id = "S-37+",
    track = "Numeric cast refinement",
    status = "research",
    inspect = "inspect shows cast policy",
    manual_rust = "use checked, saturating, or wrapping conversions explicitly",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "v14_cast_snapshot",
    replay_related = "false",
    cast_policy = "saturating|wrapping",
    debt_casts = "source_spans",
    strict_casts = "explicit"
)]"#,
        ),
        "cast_policy_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(
        fact(candidate, "cast_policy")["value"],
        "saturating|wrapping"
    );
    assert_eq!(fact(candidate, "debt_casts")["value"], "source_spans");
    assert_eq!(fact(candidate, "strict_casts")["value"], "explicit");
}
