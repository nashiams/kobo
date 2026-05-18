mod v09_common;

use std::path::Path;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

#[test]
fn sim_scout_fix_plan_covers_common_replay_boundary_classes() {
    let project = TestProject::new("v11-fix-plan");
    let file = project.main_file(
        r#"
use reqwest::Client;

async fn replay_gap() {
    let _now = std::time::SystemTime::now();
    let _random = rand::thread_rng();
    let _env = std::env::var("PAYMENTS_URL");
    let _client = Client::new();
    tokio::spawn(async {});
    let _file = std::fs::read_to_string("state.txt");
    let _process = std::process::id();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--fix-plan"),
            s("--json"),
        ],
        &project.root,
    );

    assert_success(&output, "sim scout --fix-plan should produce suggestions");
    for expected in [
        "wall-clock",
        "random",
        "environment",
        "http-database",
        "task-spawn",
        "filesystem",
        "process",
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "fix plan should classify each replay-critical source class",
        );
    }
    assert_contains(
        &output.stdout,
        "kobo.time.now",
        "wall clock fix plan should suggest replay-owned time or record policy",
    );
    assert_contains(
        &output.stdout,
        "activity",
        "external side effects should include activity as a policy option",
    );
}
