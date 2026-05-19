mod v09_common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V12_TIMEOUT: Duration = Duration::from_secs(10);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V12_TIMEOUT)
}

fn watch_project(label: &str) -> (TestProject, PathBuf, PathBuf) {
    let project = TestProject::new(label);
    let main = project.main_file(
        r#"
mod service;

fn main() {
    service::run();
}
"#,
    );
    let service = project.write(
        "src/service.kobo",
        r#"
pub fn run() {
    ward.task();
}
"#,
    );
    (project, main, service)
}

#[test]
fn watch_plan_lists_files_and_rerun_targets_without_running_forever() {
    let (project, main, _) = watch_project("watch-plan-list");
    let output = run_kobo(&[s("watch"), s("--plan"), path_arg(&main)], &project.root);

    assert_success(&output, "watch --plan should be bounded and succeed");
    let text = output.combined();
    for expected in [
        "Watch plan",
        "src/main.kobo",
        "src/service.kobo",
        "rerun targets",
        "kobo check src/main.kobo",
        "kobo inspect src/main.kobo",
    ] {
        assert_contains(
            &text,
            expected,
            "watch plan should list concrete files and rerun targets",
        );
    }
}

#[test]
fn watch_plan_invalidates_changed_service_file() {
    let (project, main, service) = watch_project("watch-plan-invalidate-service");
    let output = run_kobo(
        &[
            s("watch"),
            s("--plan"),
            path_arg(&main),
            s("--changed"),
            path_arg(&service),
        ],
        &project.root,
    );

    assert_success(&output, "watch --plan --changed should succeed");
    let text = output.combined();
    for expected in [
        "invalidated",
        "src/service.kobo",
        "rerun target: kobo check src/main.kobo",
        "rerun target: kobo inspect src/main.kobo",
    ] {
        assert_contains(
            &text,
            expected,
            "changed service file should invalidate the scoped watch target",
        );
    }
}

#[test]
fn watch_groundwork_is_disabled_for_unscoped_workspace_by_default() {
    let (project, _, _) = watch_project("watch-plan-unscoped");
    let output = run_kobo(&[s("watch"), s("--plan")], &project.root);

    assert_failure(
        &output,
        "unscoped watch planning should fail instead of scanning the workspace",
    );
    assert_contains(
        &output.combined(),
        "unscoped workspace watch is disabled",
        "unscoped watch failure should explain the required scope",
    );
}
