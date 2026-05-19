mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_mentions_line, assert_not_contains, assert_success,
    one_based_line_of, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V12_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V12_TIMEOUT)
}

fn inspect_source(label: &str, source: &str) -> String {
    let project = TestProject::new(label);
    write_tokio_manifest(&project);
    let file = project.main_file(source);
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(&output, "spawn-local fixture should inspect");
    output.combined()
}

fn write_tokio_manifest(project: &TestProject) {
    project.write(
        "Cargo.toml",
        r#"[package]
name = "spawn_local_fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
"#,
    );
}

fn first_witness(project: &TestProject) -> (PathBuf, Value) {
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    (witness_path, witness)
}

fn non_send_spawn_source(spawn_keyword: &str) -> String {
    format!(
        r#"
use std::rc::Rc;

async fn serve_local() {{
    let state = Rc::new(String::from("local-only"));
    {spawn_keyword} {{
        let _seen = state.clone();
    }};
}}
"#
    )
}

#[test]
fn spawn_local_lowers_to_localset_and_spawn_local() {
    let generated = inspect_source(
        "spawn-local-lowering",
        &non_send_spawn_source("spawn local"),
    );

    assert_contains(
        &generated,
        "tokio::task::LocalSet::new",
        "explicit local task zone should allocate a LocalSet",
    );
    assert_contains(
        &generated,
        "tokio::task::spawn_local",
        "explicit local task zone should lower through spawn_local",
    );
    assert_not_contains(
        &generated,
        "tokio::spawn(async move",
        "explicit local task zone must not lower through cross-thread spawn",
    );
}

#[test]
fn spawn_local_non_send_capture_is_allowed_inside_local_zone() {
    let project = TestProject::new("spawn-local-non-send-allowed");
    write_tokio_manifest(&project);
    let source = non_send_spawn_source("spawn local");
    let file = project.main_file(&source);

    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );

    assert_success(
        &output,
        "explicit spawn local should allow concrete Rc capture inside the local zone",
    );
    assert_contains(
        &output.combined(),
        "spawn_local",
        "inspect output should show the local scheduling primitive",
    );
}

#[test]
fn normal_spawn_rejects_non_send_capture() {
    let project = TestProject::new("normal-spawn-non-send-reject");
    write_tokio_manifest(&project);
    let source = non_send_spawn_source("spawn");
    let file = project.main_file(&source);

    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );

    assert_failure(
        &output,
        "normal spawn should reject the same concrete Rc capture",
    );
    for expected in ["K0061", "state", "Rc", "spawn local"] {
        assert_contains(
            &output.combined(),
            expected,
            "normal spawn diagnostic should explain the non-Send blocker",
        );
    }
}

#[test]
fn spawn_local_escape_is_diagnostic_with_source_span() {
    let project = TestProject::new("spawn-local-escape");
    write_tokio_manifest(&project);
    let source = r#"
async fn leak_local_task() {
    let task = spawn local {
        println!("local");
    };
    return task;
}
"#;
    let file = project.main_file(source);
    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );
    let escape_line = one_based_line_of(source, "return task");

    assert_failure(&output, "local task handle escape should be diagnostic");
    for expected in ["K0067", "task-local", "task"] {
        assert_contains(
            &output.combined(),
            expected,
            "escape diagnostic should identify the task-local handle",
        );
    }
    assert_mentions_line(
        &output,
        escape_line,
        "escape diagnostic should point at the escaping return",
    );
}

#[test]
fn spawn_local_service_cancellation_is_scenario_visible() {
    let project = TestProject::new("spawn-local-cancellation-witness");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn local_cancellation() {
    spawn local {
        ward.task();
    };
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--inject"),
            s("cancel"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "local task cancellation path should produce witness evidence",
    );
    let (_, witness) = first_witness(&project);
    let events = witness["events"].to_string();
    for expected in [
        "ward.task.local",
        "scheduler-cancel-path",
        "failure-injection-cancel",
    ] {
        assert_contains(
            &events,
            expected,
            "task-local cancellation evidence should be visible in the witness",
        );
    }
}

#[test]
fn inspect_marks_task_local_zone() {
    let generated = inspect_source(
        "spawn-local-inspect-marker",
        r#"
async fn serve_local() {
    spawn local {
        println!("local");
    };
}
"#,
    );

    assert_contains(
        &generated,
        "task-local-zone",
        "inspect output should mark the generated task-local zone",
    );
}
