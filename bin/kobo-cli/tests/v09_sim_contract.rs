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
    assert_contains(
        &text,
        "shuttle",
        "async sim init should record the selected backend profile",
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

    let scaffold = project.read(".kobo/sim/handle_request.sim.json");
    assert_contains(
        &scaffold,
        "handle_request",
        "sim init must write a target-specific scaffold artifact",
    );
    assert_contains(
        &scaffold,
        "async",
        "sim init scaffold must record the pinned profile",
    );
    assert_contains(
        &scaffold,
        "backend",
        "sim init scaffold must record backend selection metadata",
    );
    assert_contains(
        &scaffold,
        "scenario_metadata",
        "sim init scaffold must point at the generated scenario island",
    );
    let island = project.read(".kobo/sim/handle_request.scenario.kobo");
    assert_contains(
        &island,
        r#"#[kobo::scenario(profile = "async")]"#,
        "sim init must generate a real scenario metadata island",
    );
    assert_contains(
        &island,
        "handle_request",
        "scenario island must be tied to the requested target",
    );
    assert_not_contains(
        &island,
        "shuttle::",
        "scenario island must not import backend crates into Kobo source",
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
fn sim_init_generates_runnable_primitive_inputs_for_parameterized_target() {
    let project = TestProject::new("sim-init-parameterized");
    let file = project.main_file(
        r#"
async fn handle_amount(user_id: u64, dry_run: bool, label: &str) {
    if dry_run {
        println!("{}", label);
    }
    println!("{}", user_id);
}
"#,
    );

    let output = run_kobo(
        &[
            s("sim"),
            s("init"),
            s("--target"),
            format!("{}:handle_amount", path_arg(&file)),
            s("--minimal"),
            s("--profile"),
            s("async"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "sim init should generate a runnable primitive input island",
    );
    let scaffold = project.read(".kobo/sim/handle_amount.sim.json");
    assert_contains(
        &scaffold,
        "input_fixtures",
        "scaffold must record generated primitive inputs",
    );
    assert_contains(
        &scaffold,
        "user_id",
        "input fixture metadata must keep parameter names",
    );
    let island = project.read(".kobo/sim/handle_amount.scenario.kobo");
    assert_contains(
        &island,
        "handle_amount(0_u64, false, \"kobo-sim\")",
        "scenario island must call the target with generated primitive values",
    );
    assert_contains(
        &island,
        ".await",
        "async parameterized target should still be awaited",
    );
    assert_not_contains(
        &island,
        "target inputs required",
        "primitive parameters should not degrade into a comment-only island",
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
fn sim_init_recommends_each_v09_profile_from_target_shape() {
    let cases = vec![
        SimProfileCase {
            label: "sync",
            symbol: unique_symbol("handle_sync"),
            source_template: "fn __SYMBOL__() { let mut total = 0; total += 1; }",
            expected_profile: "sync",
            expected_backend: "loom",
        },
        SimProfileCase {
            label: "async",
            symbol: unique_symbol("handle_async"),
            source_template: r#"
async fn __SYMBOL__() {
    tokio::select! {
        _ = ward.task => {}
    }
}
"#,
            expected_profile: "async",
            expected_backend: "shuttle",
        },
        SimProfileCase {
            label: "stateful",
            symbol: unique_symbol("reduce_state"),
            source_template: r#"
fn __SYMBOL__() {
    let accepted = parse_operation("deposit");
    assert!(accepted);
}

fn parse_operation(input: &str) -> bool {
    input == "deposit"
}
"#,
            expected_profile: "stateful-input",
            expected_backend: "proptest",
        },
        SimProfileCase {
            label: "failpoint",
            symbol: unique_symbol("send_with_failpoint"),
            source_template: r#"
fn __SYMBOL__() {
    ward.failpoint("before-send");
}
"#,
            expected_profile: "failpoint",
            expected_backend: "failpoints",
        },
        SimProfileCase {
            label: "network",
            symbol: unique_symbol("fetch_remote"),
            source_template: r#"
async fn __SYMBOL__() {
    let _client = reqwest::Client::new();
}
"#,
            expected_profile: "network",
            expected_backend: "network-design",
        },
    ];

    for case in cases {
        let project = TestProject::new(&format!("sim-profile-{}", case.label));
        let source = case.source_template.replace("__SYMBOL__", &case.symbol);
        let file = project.write("src/profile.kobo", &source);

        let output = run_kobo(
            &[
                s("sim"),
                s("init"),
                s("--target"),
                format!("{}:{}", path_arg(&file), case.symbol),
                s("--minimal"),
            ],
            &project.root,
        );

        assert_success(&output, "sim init should infer target profile");
        let text = output.combined();
        assert_contains(
            &text,
            &format!(r#""profile": "{}""#, case.expected_profile),
            "sim init must infer the stable Kobo profile from target shape",
        );
        assert_contains(
            &text,
            &format!(r#""backend": "{}""#, case.expected_backend),
            "sim init must record the backend metadata for the inferred profile",
        );
        assert_contains(
            &text,
            &case.symbol,
            "sim init metadata must follow the dynamic target symbol",
        );
        let copied_source = project.read("src/profile.kobo");
        assert_not_contains(
            &copied_source,
            "shuttle::",
            "profile inference must not rewrite source to backend imports",
        );
        assert_not_contains(
            &copied_source,
            "loom::",
            "profile inference must not rewrite source to backend imports",
        );
    }
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
fn sim_quick_resolved_must_call_does_not_emit_k0100() {
    let project = TestProject::new("sim-liveness-resolved");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn resolved_delivery() {
    let delivery = Delivery {};
    delivery.ack();
}
"#,
    );

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

    assert_success(&output, "resolved must_call scenario should pass");
    assert_not_contains(
        &output.combined(),
        "K0100",
        "resolved obligation must not emit liveness failure",
    );
}

#[test]
fn runtime_liveness_ignores_comment_discharge_text() {
    let project = TestProject::new("sim-liveness-comment");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn unresolved_comment_delivery() {
    let delivery = Delivery {};
    // delivery.ack();
    let _lost = delivery;
}
"#,
    );

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

    assert_failure(
        &output,
        "comment-only discharge must not satisfy must_call liveness",
    );
    let text = output.combined();
    assert_contains(&text, "K0100", "unresolved token must emit K0100");
    assert_contains(
        &text,
        "delivery",
        "diagnostic should name the dropped token",
    );
}

#[test]
fn sim_quick_no_scenario_is_clear_failure() {
    let project = TestProject::new("sim-no-scenario");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");

    let output = run_kobo(
        &[s("test"), s("--sim"), s("quick"), path_arg(&file)],
        &project.root,
    );

    assert_failure(&output, "missing scenario must fail");
    let text = output.combined();
    assert_contains(&text, "scenario", "failure should name missing scenario");
    assert_contains(&text, "kobo::scenario", "failure should be actionable");
}

#[test]
fn sim_deep_is_reserved_with_actionable_message() {
    let project = TestProject::new("sim-deep-reserved");
    let file = project.copy_fixture("sim/gateway.kobo", "src/gateway.kobo");

    let output = run_kobo(
        &[s("test"), s("--sim"), s("deep"), path_arg(&file)],
        &project.root,
    );

    assert_failure(&output, "v0.9 should reserve deep simulation honestly");
    let text = output.combined();
    assert_contains(&text, "--sim deep", "message should name requested profile");
    assert_contains(&text, "v0.10", "message should point to the future phase");
    assert_contains(
        &text,
        "--sim quick",
        "message should offer the v0.9 command",
    );
}

#[test]
fn inspect_sim_uses_v09_transparency_wording() {
    let project = TestProject::new("inspect-sim-wording");
    let file = project.copy_fixture("sim/gateway.kobo", "src/gateway.kobo");

    let output = run_kobo(&[s("inspect"), s("--sim"), path_arg(&file)], &project.root);

    assert_success(&output, "inspect --sim should succeed");
    let text = output.combined();
    assert_contains(&text, "v0.9", "inspect --sim should name v0.9 scope");
    assert_contains(
        &text,
        "checked simulation MVP",
        "inspect --sim should describe the scoped v0.9 surface",
    );
    assert_not_contains(
        &text,
        "v0.8.5",
        "inspect --sim must not expose stale v0.8.5 wording",
    );
}

#[test]
fn sim_scout_why_uses_v09_backend_recommendation_wording() {
    let project = TestProject::new("sim-scout-wording");
    let file = project.copy_fixture("sim/gateway.kobo", "src/gateway.kobo");

    let output = run_kobo(
        &[s("sim"), s("scout"), s("--why"), path_arg(&file)],
        &project.root,
    );

    assert_success(&output, "sim scout --why should succeed");
    let text = output.combined();
    assert_contains(&text, "v0.9", "scout should name v0.9 scope");
    assert_contains(
        &text,
        "recommendation",
        "scout should describe backend choice as recommendation",
    );
    assert_not_contains(
        &text,
        "v0.8.5",
        "sim scout must not expose stale v0.8.5 wording",
    );
}

#[test]
fn explain_reports_backend_profile_rationale() {
    let project = TestProject::new("profile-explain");

    let output = run_kobo(&[s("explain"), s("profile:async")], &project.root);

    assert_success(
        &output,
        "kobo explain should explain stable backend profiles",
    );
    let text = output.combined();
    assert_contains(&text, "profile:async", "explain should name the profile");
    assert_contains(
        &text,
        "shuttle",
        "async profile explanation should include backend metadata",
    );
    assert_contains(
        &text,
        "tokio::spawn",
        "async profile explanation should describe why the profile is selected",
    );
    assert_contains(
        &text,
        "v0.9",
        "profile explain must be honest about v0.9 recommendation scope",
    );
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
fn uncontrolled_effect_emits_k0103() {
    let project = TestProject::new("uncontrolled-effect");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn uncontrolled_effect() {
    let _ = std::fs::read_to_string("state.txt");
}
"#,
    );

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

    assert_failure(&output, "uncontrolled effect must block exact replay");
    let text = output.combined();
    assert_contains(&text, "K0103", "uncontrolled effect must emit K0103");
    assert_contains(
        &text,
        "uncontrolled",
        "K0103 message must name uncontrolled replay effect",
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
    let changed = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("100"),
            s("--inject"),
            s("cancel,preempt,time-jump,crash"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&first, "failure-injection run should succeed");
    assert_success(&second, "same-seed failure-injection run should succeed");
    assert_success(
        &changed,
        "changed-seed failure-injection run should succeed",
    );
    assert_eq!(
        first.combined(),
        second.combined(),
        "same seed must produce the same injection order"
    );
    assert_ne!(
        first.combined(),
        changed.combined(),
        "changed seed should explore a different injection order"
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

#[test]
fn failure_injection_cancel_and_crash_change_modeled_outcome() {
    let project = TestProject::new("failure-injection-behavior");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn cancel_delivery() {
    let delivery = Delivery {};
    ward.task();
    delivery.ack();
}
"#,
    );

    let cancel = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("1"),
            s("--inject"),
            s("cancel"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &cancel,
        "cancel injection should make an active obligation fail",
    );
    let cancel_text = cancel.combined();
    assert_contains(
        &cancel_text,
        "K0100",
        "cancel injection must affect liveness, not just append metadata",
    );
    assert_contains(
        &cancel_text,
        "cancel",
        "cancel failure should preserve the injected hook label",
    );

    let crash = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("1"),
            s("--inject"),
            s("crash"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&crash, "crash injection should stop the modeled effect");
    let crash_text = crash.combined();
    assert_contains(
        &crash_text,
        "K0103",
        "crash injection must become an uncontrolled modeled failure",
    );
    assert_contains(
        &crash_text,
        "crash",
        "crash failure should preserve the injected hook label",
    );
}

struct SimProfileCase {
    label: &'static str,
    symbol: String,
    source_template: &'static str,
    expected_profile: &'static str,
    expected_backend: &'static str,
}
