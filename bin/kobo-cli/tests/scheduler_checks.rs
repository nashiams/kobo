mod cli_common;

use cli_common::{
    assert_contains, assert_failure, assert_success, first_json, path_arg, run_kobo, s, TestProject,
};

fn scheduled_source(symbol: &str) -> String {
    format!(
        r#"
#[kobo::scenario(profile = "async")]
fn {symbol}() {{
    ward.random.u64();
    ward.task();
}}
"#
    )
}

#[test]
fn quick_and_deep_are_distinct_public_scheduler_profiles() {
    let project = TestProject::new("runtime-scheduler-distinct");
    let file = project.main_file(&scheduled_source("schedule_portfolio"));

    let quick = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("7"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let deep = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("7"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&quick, "quick scheduler profile should run");
    assert_success(&deep, "deep scheduler profile should run");

    let quick_json = first_json(&quick, "quick scheduler JSON");
    let deep_json = first_json(&deep, "deep scheduler JSON");
    assert_eq!(quick_json["sim_profile"], "quick");
    assert_eq!(deep_json["sim_profile"], "deep");
    assert_ne!(
        quick_json["scheduler"], deep_json["scheduler"],
        "deep must not be an alias for quick"
    );
    assert_ne!(
        quick_json["events"], deep_json["events"],
        "profile portfolio should affect explored event stream"
    );
}

#[test]
fn exhaustive_rejects_non_tiny_ward_with_k0105() {
    let project = TestProject::new("runtime-exhaustive-budget");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn not_tiny() {
    loop {}
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("exhaustive"),
            s("--event-budget"),
            s("2"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "non-tiny exhaustive scenario must fail");
    let text = output.combined();
    assert_contains(&text, "K0105", "exhaustive cap must emit K0105");
    assert_contains(
        &text,
        "exhaustive",
        "budget diagnostic should name the scheduler profile",
    );
}

#[test]
fn concurrent_test_attribute_routes_into_scheduler_presets() {
    let project = TestProject::new("runtime-concurrent-test-bridge");
    let file = project.main_file(
        r#"
#[kobo::concurrent_test]
fn counter_bridge() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("37"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "concurrent_test should bridge into runtime scheduler presets",
    );
    let json = first_json(&output, "concurrent scheduler JSON");
    assert_eq!(json["sim_profile"], "deep");
    assert_eq!(json["backend_profile"], "sync");
    assert_contains(
        &json["events"].to_string(),
        "scheduler-pct-seed",
        "deep concurrent test should use the deep scheduler portfolio",
    );
}

#[test]
fn scheduler_records_runnable_wakeup_drop_and_cancel_state() {
    let project = TestProject::new("runtime-scheduler-state-machine");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn async_gateway() {
    let reply = ReplyToken {};
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("41"),
            s("--inject"),
            s("cancel"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "events=json should expose the cancelled async scheduler path",
    );
    let json = first_json(&output, "cancel scheduler JSON");
    let events = json["events"].to_string();
    for expected in [
        "scheduler-task-enqueued",
        "scheduler-task-wakeup",
        "scheduler-task-polled",
        "scheduler-runnable-queue",
        "scheduler-cancel-path",
        "scheduler-future-dropped",
    ] {
        assert_contains(
            &events,
            expected,
            "scheduler model must expose runnable queue, wakeup, poll, cancellation, and dropped-future state",
        );
    }
}
