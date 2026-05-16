mod v09_common;

use std::fs;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_not_contains, assert_success, first_json, path_arg, run_kobo, s,
    TestProject,
};

#[test]
fn fuzz_runs_stateful_scenario_cases_on_top_of_scheduler_profile() {
    let project = TestProject::new("v10-fuzz-stateful");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "stateful-input")]
fn generated_queue_ops() {
    ward.random.u64();
    ward.storage.write("message");
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--fuzz"),
            s("--seed"),
            s("41"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "fuzzed stateful-input scenario should run");
    let json = first_json(&output, "fuzz event JSON");
    assert_eq!(json["sim_profile"], "deep");
    assert_eq!(json["backend_profile"], "stateful-input");
    assert_eq!(json["fuzz"]["runner"], "proptest");
    assert_eq!(json["fuzz"]["strategy"], "proptest-stateful-input");
    assert_contains(
        &json["events"].to_string(),
        "fuzz-case",
        "event stream should include generated fuzz cases",
    );
    assert_contains(
        &json["events"].to_string(),
        "stateful-input-op",
        "fuzzing should generate stateful scenario operations, not only rerun seeds",
    );
    assert_contains(
        &json["events"].to_string(),
        "ward.storage.write",
        "generated fuzz operations should target modeled scenario data sources",
    );
    assert_contains(
        &json["events"].to_string(),
        "fuzz-shrink-candidate",
        "fuzzing should expose shrink candidates for replay-preserving minimization",
    );
    assert_contains(
        &json["events"].to_string(),
        "seed=41",
        "fuzz metadata should be derived from the requested base seed",
    );
}

#[test]
fn fuzz_harness_agreement_is_independent() {
    let project = TestProject::new("v10-fuzz-independent");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "stateful-input")]
fn generated_queue_ops() {
    ward.random.u64();
    ward.storage.write("message");
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--fuzz"),
            s("--seed"),
            s("41"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "fuzzed stateful-input witness should write");
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("fuzz witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    let events = witness["events"].to_string();
    assert_contains(
        &events,
        "fuzz-case",
        "witness should retain the fuzz-driver event stream",
    );
    assert_contains(
        &events,
        "stateful-input-op",
        "witness should retain generated stateful input operations",
    );
    assert_eq!(witness["replay_guarantee"].as_str(), Some("partial"));
    assert_eq!(witness["exactness"].as_str(), Some("partial"));
    let fuzz_driver_trace_hash = witness["execution_digest"]["fuzz_driver_trace_hash"]
        .as_str()
        .expect("fuzz-driver evidence must have its own digest");
    assert!(
        !fuzz_driver_trace_hash.is_empty(),
        "fuzz-driver evidence must have its own digest: {witness}"
    );
    assert_ne!(
        witness["execution_digest"]["semantic_trace_hash"].as_str(),
        Some(fuzz_driver_trace_hash),
        "CLI fuzz aggregation must not replace the semantic trace hash"
    );
    assert_ne!(
        witness["execution_digest"]["harness_trace_hash"].as_str(),
        Some(fuzz_driver_trace_hash),
        "CLI fuzz aggregation must not replace the generated-harness trace hash"
    );
}

#[test]
fn concurrent_state_sugar_lowers_through_compiler_owned_generated_rust() {
    let project = TestProject::new("v10-concurrent-sugar");
    let file = project.main_file(
        r#"
fn state_surface() {
    #[kobo::shared]
    struct SharedState {
        #[kobo::counter]
        hits: u64,
        #[kobo::live]
        route: String,
        ordinary: String,
    }

    let state = SharedState {
        hits: 0,
        route: String::from("blue"),
        ordinary: String::from("plain"),
    };
    let _state_route = state.route.load();

    @counter hits: u64 = 0;
    hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    @live route: String = String::from("blue");
    let _loaded_route = route.load();

    @view_distance radius: u32 = 128;
    let _radius_cells = radius.get();

    #[kobo::shared]
    let shared_routes: Vec<String> = Vec::new();
    let _route_count = shared_routes.len();

    let small: u8 = 7;
    let widened: u64 = small;
    let _widened = widened;

    let mut cells = vec![1_u32, 2_u32];
    for cell in cells.iter() {
        cells.push(cell);
    }

    #[kobo::critical_section]
    {
        let _inside = cells.len();
    }
}
"#,
    );

    let output = run_kobo(
        &[s("build"), s("--emit-rust"), path_arg(&file)],
        &project.root,
    );

    assert_success(
        &output,
        "Phase 10 concurrent-state sugar should lower through build --emit-rust",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "AtomicU64::new(0)",
        "@counter should lower to AtomicU64",
    );
    assert_contains(
        &text,
        "KoboArcSwap::from_pointee",
        "@live should lower to an ArcSwap-compatible generated cell",
    );
    assert_contains(
        &text,
        "arc_swap::ArcSwap",
        "@live should use the real ArcSwap backend rather than a lock-backed shim",
    );
    assert_not_contains(
        &text,
        "RwLock<std::sync::Arc",
        "@live should not claim wait-free reads while using an RwLock-backed cell",
    );
    assert_contains(
        &text,
        "KoboViewDistance::new(128)",
        "@view_distance should lower to an inspectable helper",
    );
    assert_contains(
        &text,
        "Arc::new(Vec::new())",
        "#[kobo::shared] should force shared solver/codegen lowering",
    );
    assert_contains(
        &text,
        "hits: std::sync::atomic::AtomicU64",
        "#[kobo::shared] struct fields should support per-field counter wrapping",
    );
    assert_contains(
        &text,
        "route: KoboArcSwap<String>",
        "#[kobo::shared] struct fields should support per-field live-cell wrapping",
    );
    assert_contains(
        &text,
        "hits: std::sync::atomic::AtomicU64::new(0)",
        "struct initializers should wrap counter fields, not leave invalid raw values",
    );
    assert_contains(
        &text,
        "route: KoboArcSwap::from_pointee(String::from(\"blue\"))",
        "struct initializers should wrap live fields, not leave invalid raw values",
    );
    assert_contains(
        &text,
        "small as u64",
        "script-mode numeric widening should insert an explicit cast",
    );
    assert_contains(
        &text,
        "numeric-cast-debt",
        "numeric widening should leave an inspectable debt note for review",
    );
    assert_contains(
        &text,
        "__kobo_iter_snapshot",
        "iterator mutation conflict should materialize a snapshot boundary",
    );
    assert_contains(
        &text,
        "iterator-materialization-debt",
        "iterator materialization should leave an inspectable debt note",
    );
    assert_contains(
        &text,
        "__kobo_critical_section",
        "critical section lowering should be visible in generated Rust",
    );

    let source = project.read("src/main.kobo");
    for backend_import in [
        "loom::",
        "shuttle::",
        "turmoil::",
        "madsim::",
        "failpoints::",
    ] {
        assert_not_contains(
            &source,
            backend_import,
            "state ergonomics must not require backend-shaped user imports",
        );
    }
}

#[test]
fn k0062_guard_across_await_gives_lock_scope_refactoring_suggestion() {
    let project = TestProject::new("v10-k0062-refactor");
    let file = project.main_file(
        r#"
async fn guard_lifetime() {
    let state_guard = 1_u32;
    wait_for_io().await;
    let _after = state_guard;
}

async fn wait_for_io() {}
"#,
    );

    let output = run_kobo(&[s("check"), path_arg(&file)], &project.root);

    assert_success(&output, "guard liveness diagnostic is warning-level");
    let text = output.combined();
    assert_contains(&text, "K0062", "diagnostic code should be K0062");
    assert_contains(
        &text,
        "guard before .await",
        "K0062 should explain the guard lifetime problem",
    );
    assert_contains(
        &text,
        "block scope",
        "K0062 should suggest a lock-scope split/refactor",
    );
}

#[test]
fn live_cell_build_compiles_without_user_arc_swap_dependency() {
    let project = TestProject::new("v10-live-cell-build");
    let file = project.main_file(
        r#"
fn main() {
    @live route: String = String::from("blue");
    let _loaded = route.load();
}
"#,
    );

    let output = run_kobo(&[s("build"), path_arg(&file)], &project.root);

    assert_success(
        &output,
        "real ArcSwap-backed live cell should compile through Kobo's rustc path",
    );
    let source = project.read("src/main.kobo");
    assert_not_contains(
        &source,
        "arc_swap",
        "user source must not require the generated live-cell backend dependency",
    );
}

#[test]
fn self_referential_struct_fix_proposes_box_restructure() {
    let project = TestProject::new("v10-self-ref-fix");
    let file = project.main_file(
        r#"
struct Node {
    next: Node,
    value: u32,
}
"#,
    );

    let output = run_kobo(
        &[s("fix"), s("--dry-run"), s("--json"), path_arg(&file)],
        &project.root,
    );

    assert_success(&output, "self-referential fix planning should succeed");
    let text = output.combined();
    assert_contains(
        &text,
        "self-referential",
        "fix output should name the structural problem",
    );
    assert_contains(
        &text,
        "next: Box<Node>",
        "fix output should propose an explicit indirection",
    );
}
