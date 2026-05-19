mod v09_common;

use std::path::Path;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
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

extern "C" {
    fn ffi_ping();
}

async fn replay_gap() {
    let _now = std::time::SystemTime::now();
    let _random = rand::thread_rng();
    let _env = std::env::var("PAYMENTS_URL");
    let _client = Client::new();
    tokio::spawn(async {});
    let _file = std::fs::read_to_string("state.txt");
    let _socket = std::net::TcpStream::connect("127.0.0.1:8080");
    let _process = std::process::id();
    std::thread::sleep(std::time::Duration::from_millis(1));
    unsafe { ffi_ping(); }
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
        "socket",
        "process",
        "ffi",
        "observable-scheduling",
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
    assert_contains(
        &output.stdout,
        "source_spans",
        "fix plan should include source span evidence for replay-critical classes",
    );
    assert_contains(
        &output.stdout,
        "call_path",
        "fix plan should include call path evidence when a boundary call is resolved",
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("fix plan should be JSON");
    let http_item = parsed["fix_plan"]
        .as_array()
        .expect("fix plan should be an array")
        .iter()
        .find(|item| item["class"] == "http-database")
        .expect("http/database item should exist");
    assert_contains(
        &http_item.to_string(),
        "reqwest::Client::new",
        "fix plan should carry the resolved call path from parsed source",
    );
}

#[test]
fn test_sim_flags_environment_process_and_sleep_on_replay_path() {
    for (label, source, expected) in [
        (
            "env",
            r#"
#[kobo::scenario(profile = "async")]
fn replay_gap() {
    let _env = std::env::var("PAYMENTS_URL");
}
"#,
            "std::env",
        ),
        (
            "process",
            r#"
#[kobo::scenario(profile = "async")]
fn replay_gap() {
    let _pid = std::process::id();
}
"#,
            "std::process",
        ),
        (
            "sleep",
            r#"
#[kobo::scenario(profile = "async")]
fn replay_gap() {
    std::thread::sleep(std::time::Duration::from_millis(1));
}
"#,
            "std::thread::sleep",
        ),
        (
            "imported-env",
            r#"
use std::env::var;

#[kobo::scenario(profile = "async")]
fn replay_gap() {
    let _env = var("PAYMENTS_URL");
}
"#,
            "std::env",
        ),
        (
            "imported-process",
            r#"
use std::process::id;

#[kobo::scenario(profile = "async")]
fn replay_gap() {
    let _pid = id();
}
"#,
            "std::process",
        ),
        (
            "imported-sleep",
            r#"
use std::thread::sleep;

#[kobo::scenario(profile = "async")]
fn replay_gap() {
    sleep(std::time::Duration::from_millis(1));
}
"#,
            "std::thread::sleep",
        ),
        (
            "block-imported-env",
            r#"
#[kobo::scenario(profile = "async")]
fn replay_gap() {
    use std::env::var;
    let _env = var("PAYMENTS_URL");
}
"#,
            "std::env",
        ),
        (
            "block-imported-process",
            r#"
#[kobo::scenario(profile = "async")]
fn replay_gap() {
    use std::process::id;
    let _pid = id();
}
"#,
            "std::process",
        ),
        (
            "block-imported-sleep",
            r#"
#[kobo::scenario(profile = "async")]
fn replay_gap() {
    use std::thread::sleep;
    sleep(std::time::Duration::from_millis(1));
}
"#,
            "std::thread::sleep",
        ),
    ] {
        let project = TestProject::new(&format!("v11-test-sim-replay-nondeterminism-{label}"));
        let file = project.main_file(source);

        let output = run_kobo(
            &[s("test"), s("--sim"), s("quick"), path_arg(&file)],
            &project.root,
        );

        assert_failure(
            &output,
            "test --sim should reject replay-path nondeterminism classes",
        );
        assert_contains(
            &output.combined(),
            expected,
            "replay-path nondeterminism failure should name the offending operation",
        );
    }
}

#[test]
fn sim_scout_fix_plan_ignores_comments_strings_and_local_same_name_types() {
    let project = TestProject::new("v11-fix-plan-ast-evidence");
    let file = project.main_file(
        r#"
// reqwest::Client::new in a comment is not executable evidence.
const NOTE: &str = "sqlx::query in a string is not executable evidence";

struct Client;

impl Client {
    fn new() -> Self {
        Client
    }
}

fn local_only() {
    let _client = Client::new();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--json"),
            s("--fix-plan"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "sim scout --fix-plan should inspect parsed executable source",
    );
    assert!(
        !output.stdout.contains("http-database"),
        "comments, strings, and local Client::new must not become HTTP/database evidence:\n{}",
        output.stdout
    );
}

#[test]
fn sim_scout_fix_plan_ignores_local_same_name_functions_modules_and_methods() {
    let project = TestProject::new("v11-fix-plan-local-name-false-positives");
    let file = project.main_file(
        r#"
fn thread_rng() {}

mod sqlx {
    pub fn query() {}
}

mod reqwest {
    pub fn get() {}
}

struct Worker;

impl Worker {
    fn spawn(&self) {}
    fn sleep(&self) {}
    fn yield_now(&self) {}
}

fn local_only(worker: Worker) {
    thread_rng();
    sqlx::query();
    reqwest::get();
    worker.spawn();
    worker.sleep();
    worker.yield_now();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--json"),
            s("--fix-plan"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "sim scout --fix-plan should parse local same-name calls without external false positives",
    );
    for blocked in [
        "random",
        "http-database",
        "task-spawn",
        "observable-scheduling",
    ] {
        assert!(
            !output.stdout.contains(blocked),
            "local same-name functions/modules/methods must not create {blocked} evidence:\n{}",
            output.stdout
        );
    }
}

#[test]
fn sim_scout_fix_plan_resolves_block_local_scopes_before_classifying() {
    let project = TestProject::new("v11-fix-plan-block-scope");
    let file = project.main_file(
        r#"
fn local_only() {
    mod sqlx {
        pub fn query() {}
    }

    mod reqwest {
        pub fn get() {}
    }

    mod local {
        pub fn thread_rng() {}

        pub struct Tokio;

        impl Tokio {
            pub fn spawn(&self) {}
            pub fn sleep(&self) {}
            pub fn yield_now(&self) {}
        }
    }

    use local::thread_rng;

    let tokio = local::Tokio;
    thread_rng();
    sqlx::query();
    reqwest::get();
    tokio.spawn();
    tokio.sleep();
    tokio.yield_now();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--json"),
            s("--fix-plan"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "sim scout --fix-plan should understand local block scopes",
    );
    for blocked in [
        "random",
        "http-database",
        "task-spawn",
        "observable-scheduling",
    ] {
        assert!(
            !output.stdout.contains(blocked),
            "block-local declarations and local shadowing must not create {blocked} evidence:\n{}",
            output.stdout
        );
    }
}

#[test]
fn sim_scout_fix_plan_resolves_block_local_external_imports() {
    let project = TestProject::new("v11-fix-plan-block-imports");
    let file = project.main_file(
        r#"
fn replay_gap() {
    use rand::thread_rng;
    use sqlx::query;
    use std::thread::{sleep, spawn};
    use std::time::Duration;
    use tokio::task::yield_now;

    let _random = thread_rng();
    let _row = query("select 1");
    spawn(|| {});
    sleep(Duration::from_millis(1));
    yield_now();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--json"),
            s("--fix-plan"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "sim scout --fix-plan should resolve block-local imports to external roots",
    );
    for expected in [
        "random",
        "http-database",
        "task-spawn",
        "observable-scheduling",
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "block-local external imports should still produce replay-critical evidence",
        );
    }
}

#[test]
fn sim_scout_fix_plan_uses_configured_registry_and_adapter_roots() {
    let project = TestProject::new("v11-fix-plan-configured-ecosystem-roots");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "opaque"

[[ecosystem.crate]]
name = "payments"
policy = "record"
reason = "configured payment gateway boundary"

[[ecosystem.adapter]]
crate = "emailer"
package = "kobo-adapter-emailer"
reason = "configured email adapter"
"#,
    );
    let file = project.main_file(
        r#"
use payments::charge;
use emailer::send;

fn replay_gap() {
    let _payment = charge();
    let _mail = send();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--json"),
            s("--fix-plan"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "sim scout --fix-plan should include configured ecosystem roots",
    );
    assert_contains(
        &output.stdout,
        "http-database",
        "configured registry/adapter roots should be treated as replay-critical boundaries",
    );
    for expected in ["payments::charge", "emailer::send"] {
        assert_contains(
            &output.stdout,
            expected,
            "configured ecosystem root evidence should retain resolved call paths",
        );
    }
}

#[test]
fn sim_scout_fix_plan_reports_invalid_ecosystem_config_instead_of_dropping_roots() {
    let project = TestProject::new("v11-fix-plan-invalid-config");
    project.write(
        "Kobo.toml",
        r#"[ecosystem]
default = "definitely-not-a-policy"

[[ecosystem.crate]]
name = "payments"
policy = "record"
"#,
    );
    let file = project.main_file(
        r#"
use payments::charge;

fn replay_gap() {
    let _payment = charge();
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("scout"),
            path_arg(&file),
            s("--json"),
            s("--fix-plan"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "sim scout --fix-plan must not silently ignore invalid configured ecosystem roots",
    );
    assert_contains(
        &output.combined(),
        "definitely-not-a-policy",
        "config load errors should be visible instead of producing an incomplete fix plan",
    );
}
