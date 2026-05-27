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

fn source(target: &str, attr: &str, body: &str) -> String {
    format!(
        r#"
{attr}
#[kobo::scenario(profile = "sync")]
fn {target}() {{
{body}
}}
"#
    )
}

fn write_config(project: &TestProject, contents: &str) {
    project.write("Kobo.toml", contents);
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
    assert_failure(&output, "invalid spike admission should fail");
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
fn target_rust_and_nightly_avoidance_are_admission_facts() {
    let project = TestProject::new("proof-msrv-target");
    write_config(
        &project,
        r#"
[output]
target_rust = "1.70"
"#,
    );
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
    diagnostic_snapshot = "proof_msrv_snapshot",
    replay_related = "false"
)]"#,
            "    let _value = 1u32;",
        ),
        "msrv_target_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "target_rust")["value"], "1.70");
    assert_eq!(fact(candidate, "avoids_nightly")["value"], "true");

    let inspect_output = run_kobo(
        &[
            s("inspect"),
            path_arg(&project.root.join("src/main.kobo")),
            s("--clean"),
        ],
        &project.root,
    );
    assert_success(&inspect_output, "inspect should expose generated Rust");
    assert!(
        !inspect_output.combined().contains("#![feature"),
        "MSRV-targeted output must avoid nightly feature gates\n{}",
        inspect_output.combined()
    );
}

#[test]
fn no_std_hidden_heap_and_memory_budget_are_admission_facts() {
    let project = TestProject::new("proof-nostd-budget");
    write_config(
        &project,
        r#"
[output]
no_std = true
memory_budget = "static-4kb"
"#,
    );
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
    diagnostic_snapshot = "proof_nostd_snapshot",
    replay_related = "false"
)]"#,
            "    let buffer = [0u8; 16];\n    let _value = buffer[0];",
        ),
        "nostd_budget_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "no_hidden_heap")["value"], "true");
    assert_eq!(fact(candidate, "allocation_report")["value"], "structural");
    assert_eq!(fact(candidate, "memory_budget")["value"], "static-4kb");
}

#[test]
fn hidden_heap_blocks_graduated_no_std_candidate() {
    let project = TestProject::new("proof-nostd-hidden-heap-fails");
    write_config(
        &project,
        r#"
[output]
no_std = true
memory_budget = "static-4kb"
"#,
    );

    emit_failure(
        &project,
        &source(
            "hidden_heap_case",
            r#"#[kobo::candidate_track(
    id = "S-49",
    track = "no_std allocation budget",
    status = "graduate",
    inspect = "inspect shows allocation report",
    manual_rust = "write no_std code with explicit buffers",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_nostd_snapshot",
    replay_related = "false"
)]"#,
            "    let values = Vec::<u8>::new();\n    let _value = values.len();",
        ),
        "hidden_heap_case",
        "hidden heap",
    );
}

#[test]
fn hidden_heap_with_capacity_blocks_graduated_no_std_candidate() {
    let project = TestProject::new("proof-nostd-hidden-heap-capacity-fails");
    write_config(
        &project,
        r#"
[output]
no_std = true
memory_budget = "static-4kb"
"#,
    );

    emit_failure(
        &project,
        &source(
            "hidden_heap_capacity_case",
            r#"#[kobo::candidate_track(
    id = "S-49",
    track = "no_std allocation budget",
    status = "graduate",
    inspect = "inspect shows allocation report",
    manual_rust = "write no_std code with explicit buffers",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_nostd_snapshot",
    replay_related = "false"
)]"#,
            "    let values = Vec::<u8>::with_capacity(4);\n    let _value = values.len();",
        ),
        "hidden_heap_capacity_case",
        "hidden heap",
    );
}

#[test]
fn cast_policy_and_strict_cast_debt_are_admission_facts() {
    let project = TestProject::new("proof-cast-policy");
    write_config(
        &project,
        r#"
[casts]
policy = "saturating|wrapping"
"#,
    );
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
    diagnostic_snapshot = "proof_cast_snapshot",
    replay_related = "false"
)]"#,
            "    let value = 7u32;\n    let _wide = value as u64;",
        ),
        "cast_policy_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(
        fact(candidate, "cast_policy")["value"],
        "saturating|wrapping"
    );
    assert_eq!(fact(candidate, "debt_casts")["value"], "source_spans");
    assert_eq!(
        fact(candidate, "strict_casts")["value"],
        "raw-casts-present"
    );

    let debt_output = run_kobo(
        &[
            s("debt"),
            path_arg(&project.root.join("src/main.kobo")),
            s("--casts"),
            s("--json"),
        ],
        &project.root,
    );
    assert_success(&debt_output, "debt --casts should report cast debt");
    let cast_report: Value =
        serde_json::from_str(&debt_output.stdout).expect("cast debt output should be JSON");
    assert_eq!(cast_report["mode"], "cast_debt");
    assert!(
        cast_report["casts"].as_array().is_some_and(|casts| {
            casts.iter().any(|cast| {
                cast["policy"].as_str() == Some("saturating|wrapping")
                    && cast["line"].as_u64().unwrap_or_default() > 0
            })
        }),
        "cast debt should expose source spans and policy: {cast_report}"
    );
}

#[test]
fn raw_casts_block_graduated_strict_cast_candidate() {
    let project = TestProject::new("proof-strict-casts-fail");
    write_config(
        &project,
        r#"
[casts]
policy = "saturating|wrapping"
"#,
    );

    emit_failure(
        &project,
        &source(
            "raw_cast_graduate_case",
            r#"#[kobo::candidate_track(
    id = "S-37+",
    track = "Numeric cast refinement",
    status = "graduate",
    inspect = "inspect shows cast policy",
    manual_rust = "use checked, saturating, or wrapping conversions explicitly",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_cast_snapshot",
    replay_related = "false"
)]"#,
            "    let value = 7u32;\n    let _wide = value as u64;",
        ),
        "raw_cast_graduate_case",
        "explicit cast",
    );
}
