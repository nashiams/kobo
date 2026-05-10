mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, fixture_text, path_arg,
    run_kobo, s, unique_symbol, TestProject,
};

#[test]
fn sim_init_generates_one_tiny_island_and_records_profile() {
    let project = TestProject::new("sim-init");
    let file = project.copy_fixture("sim/gateway.kobo", "src/gateway.kobo");

    let output = run_kobo(
        &[
            s("sim"),
            s("init"),
            s("--target"),
            format!("{}:handle_request", path_arg(&file)),
            s("--minimal"),
            s("--profile"),
            s("async"),
        ],
        &project.root,
    );

    assert_success(&output, "sim init should succeed");
    let text = output.combined();
    assert_contains(&text, "async", "sim init should record pinned profile");
    assert_contains(
        &text,
        "handle_request",
        "sim init must target requested symbol",
    );
    assert_not_contains(
        &project.read("src/gateway.kobo"),
        "shuttle::",
        "user source must not gain backend imports",
    );
    assert_not_contains(
        &project.read("src/gateway.kobo"),
        "loom::",
        "user source must not gain backend imports",
    );
}

#[test]
fn sim_init_target_mutation_changes_generated_metadata() {
    let project = TestProject::new("sim-init-mutation");
    let file = project.write(
        "src/gateway.kobo",
        &fixture_text("sim/gateway.kobo").replace("handle_request", "handle_payment"),
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("init"),
            s("--target"),
            format!("{}:handle_payment", path_arg(&file)),
            s("--minimal"),
        ],
        &project.root,
    );

    assert_success(&output, "sim init should succeed for mutated target");
    let text = output.combined();
    assert_contains(
        &text,
        "handle_payment",
        "metadata must follow target mutation",
    );
    assert_not_contains(
        &text,
        "handle_request",
        "metadata must not be hardcoded to old target",
    );
}

#[test]
fn sim_init_selects_dynamic_target_without_leaking_neighbor() {
    let project = TestProject::new("sim-init-dynamic");
    let selected = unique_symbol("handle_selected");
    let neighbor = unique_symbol("handle_neighbor");
    let file = project.copy_fixture_template(
        "sim/dynamic_gateway.template.kobo",
        "src/dynamic_gateway.kobo",
        &[("__NEIGHBOR__", &neighbor), ("__SELECTED__", &selected)],
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("init"),
            s("--target"),
            format!("{}:{selected}", path_arg(&file)),
            s("--minimal"),
            s("--profile"),
            s("async"),
        ],
        &project.root,
    );

    assert_success(&output, "sim init should resolve the exact dynamic target");
    let text = output.combined();
    assert_contains(
        &text,
        &selected,
        "generated metadata must name requested symbol",
    );
    assert_not_contains(
        &text,
        &neighbor,
        "generated metadata must not be a canned neighbor scan",
    );
    assert_not_contains(
        &project.read("src/dynamic_gateway.kobo"),
        "shuttle::",
        "dynamic source must remain backend-agnostic",
    );
    assert_not_contains(
        &project.read("src/dynamic_gateway.kobo"),
        "loom::",
        "dynamic source must remain backend-agnostic",
    );
}

#[test]
fn sim_quick_liveness_failure_emits_k0100_and_witness() {
    let project = TestProject::new("sim-liveness");
    let file = project.copy_fixture("sim/gateway.kobo", "src/gateway.kobo");

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--profile"),
            s("checked"),
            s("--seed"),
            s("7"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "unresolved must_call scenario should fail");
    let text = output.combined();
    assert_contains(&text, "K0100", "liveness token drop must emit K0100");
    assert_contains(
        &text,
        "ack",
        "diagnostic must include discharge alternatives",
    );
    assert_contains(&text, ".kwit", "sim failure must expose witness path");
}

#[test]
fn deterministic_time_random_same_seed_replays_and_changed_seed_changes_events() {
    let project = TestProject::new("sim-deterministic");
    let file = project.copy_fixture("sim/deterministic.kobo", "src/deterministic.kobo");

    let first = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("42"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let second = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("42"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let changed = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("43"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&first, "first deterministic run should succeed");
    assert_success(&second, "same-seed deterministic run should succeed");
    assert_success(&changed, "changed-seed deterministic run should succeed");
    assert_eq!(
        first.combined(),
        second.combined(),
        "same seed must replay event stream exactly"
    );
    assert_ne!(
        first.combined(),
        changed.combined(),
        "changed seed must alter random-dependent event stream"
    );
}

#[test]
fn raw_clock_on_replay_path_emits_k0102() {
    let project = TestProject::new("raw-clock");
    let file = project.copy_fixture("sim/raw_clock.kobo", "src/main.kobo");

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "raw clock on replay path should fail");
    let text = output.combined();
    assert_contains(&text, "K0102", "raw nondeterminism must emit K0102");
    assert_contains(
        &text,
        "deterministic",
        "diagnostic must offer deterministic facade",
    );
}

#[test]
fn failure_injection_hooks_are_distinct_and_seed_deterministic() {
    let project = TestProject::new("failure-injection");
    let file = project.copy_fixture("sim/failure_injection.kobo", "src/main.kobo");

    let first = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("99"),
            s("--inject"),
            s("cancel,preempt,time-jump,crash"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let second = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("99"),
            s("--inject"),
            s("cancel,preempt,time-jump,crash"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&first, "failure-injection run should succeed");
    assert_success(&second, "same-seed failure-injection run should succeed");
    assert_eq!(
        first.combined(),
        second.combined(),
        "same seed must produce the same injection order"
    );
    let text = first.combined();
    for hook in ["cancel", "preempt", "time-jump", "crash"] {
        assert_contains(
            &text,
            hook,
            "each failure hook must be distinct in event stream",
        );
    }
}
