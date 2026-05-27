mod cli_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_common::{
    assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const V14_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V14_TIMEOUT)
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
    assert_failure(&output, "invalid research spike admission should fail");
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
fn smt_and_advanced_temporal_tracks_stay_over_stable_semantics() {
    let project = TestProject::new("proof-smt-temporal");
    let path = emit(
        &project,
        &source(
            "smt_temporal_case",
            r#"#[kobo::ward]
#[kobo::invariant]
#[kobo::temporal(query = "beyond_always_eventually_never")]
#[kobo::candidate_track(
    id = "research-smt-temporal",
    track = "SMT and advanced temporal logic",
    status = "research",
    inspect = "inspect shows stable ward invariant assumptions",
    manual_rust = "write ward invariants and trace checks manually",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_smt_temporal_snapshot",
    replay_related = "false"
)]"#,
            "    let _value = 1u32;",
        ),
        "smt_temporal_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(
        fact(candidate, "stable_semantics")["value"],
        "ward|invariant"
    );
    assert_eq!(
        fact(candidate, "temporal_extension")["value"],
        "beyond_always_eventually_never"
    );
}

#[test]
fn broad_adapters_are_curated_and_demand_driven() {
    let project = TestProject::new("proof-broad-adapters");
    write_config(
        &project,
        r#"
[adapters]
scope = "curated_demand"
treadmill = "rejected"
"#,
    );
    let path = emit(
        &project,
        &source(
            "broad_adapters_case",
            r#"#[kobo::candidate_track(
    id = "research-broad-adapters",
    track = "Broad ecosystem adapters",
    status = "research",
    inspect = "inspect shows curated adapter assumptions",
    manual_rust = "write explicit boundary policies for selected crates",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_broad_adapter_snapshot",
    replay_related = "true"
)]"#,
            "    let _value = 1u32;",
        ),
        "broad_adapters_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(fact(candidate, "adapter_scope")["value"], "curated_demand");
    assert_eq!(fact(candidate, "adapter_treadmill")["value"], "rejected");
    assert!(
        candidate["replay_grade"].as_str().is_some(),
        "replay-related adapter research must expose replay grade: {candidate}"
    );
}

#[test]
fn witness_minimization_and_model_checking_remain_optional_assumption_backed() {
    let project = TestProject::new("proof-model-checking");
    let path = emit(
        &project,
        &source(
            "model_checking_case",
            r#"#[kobo::witness_minimization(labeled_trace)]
#[kobo::model_check(backend_user_theory = "kobo_core_loop", backend_assumptions = "ledger")]
#[kobo::candidate_track(
    id = "research-model-checking",
    track = "Witness minimization and model checking backends",
    status = "research",
    inspect = "inspect shows labeled minimized trace assumptions",
    manual_rust = "run the backend separately and keep Kobo trace labels",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_model_checking_snapshot",
    replay_related = "true"
)]"#,
            "    let _value = 1u32;",
        ),
        "model_checking_case",
    );
    let candidate = &artifact(&path)["candidate_admission"][0];

    assert_eq!(
        fact(candidate, "minimization_proof")["value"],
        "labeled_trace"
    );
    assert_eq!(
        fact(candidate, "backend_user_theory")["value"],
        "kobo_core_loop"
    );
    assert_eq!(fact(candidate, "backend_assumptions")["value"], "ledger");
}

#[test]
fn backend_specific_theory_blocks_graduated_model_checking_candidate() {
    let project = TestProject::new("proof-model-checking-theory-fails");

    emit_failure(
        &project,
        &source(
            "backend_specific_theory_case",
            r#"#[kobo::witness_minimization(labeled_trace)]
#[kobo::model_check(backend_user_theory = "z3_smt", backend_assumptions = "ledger")]
#[kobo::candidate_track(
    id = "research-model-checking",
    track = "Witness minimization and model checking backends",
    status = "graduate",
    inspect = "inspect shows labeled minimized trace assumptions",
    manual_rust = "run the backend separately and keep Kobo trace labels",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_model_checking_snapshot",
    replay_related = "true"
)]"#,
            "    let _value = 1u32;",
        ),
        "backend_specific_theory_case",
        "Kobo Core theory",
    );
}
