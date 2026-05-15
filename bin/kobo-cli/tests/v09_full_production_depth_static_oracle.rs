use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"))
}

#[test]
fn production_depth_simulation_is_not_cli_local_syn_lowering() {
    let commands = read("src/commands.rs");
    let sim_model = read("src/commands/sim_model.rs");
    let sim_semantic = read("src/commands/sim_semantic.rs");
    let test_cmd = read("src/commands/test_cmd.rs");
    let replay = read("src/commands/replay.rs");

    assert!(
        !commands.contains("mod sim_semantic;"),
        "full production-depth must move CLI-local semantic lowering into kobo-sim-core"
    );
    assert!(
        !sim_semantic.contains("syn::parse_file"),
        "CLI-local syn parsing is not compiler-owned Kobo semantics"
    );
    assert!(
        !test_cmd.contains("sim_model::run_quick_target("),
        "kobo test must execute kobo-sim-core, not the old CLI sim model"
    );
    assert!(
        !replay.contains("sim_model::run_quick_target("),
        "replay cannot re-run only the same CLI model as independent evidence"
    );
    assert!(
        sim_model.contains("kobo_sim_core"),
        "sim_model compatibility layer must delegate semantics to kobo-sim-core"
    );
}

#[test]
fn production_depth_error_policy_is_not_post_codegen_text_rewrite() {
    let build_rs = read("src/commands/build.rs");
    for banned in [
        "question_operator_offsets",
        "expression_context_before",
        "rewrite_question_error_sites",
        "replace(\"std::io::Error\"",
    ] {
        assert!(
            !build_rs.contains(banned),
            "full production-depth error policy must not use text helper `{banned}`"
        );
    }
    assert!(
        build_rs.contains("error_policy_sites"),
        "build must consume compiler-owned error policy sites"
    );
}

#[test]
fn full_depth_harness_must_not_copy_semantic_hash() {
    let harness = read("../../crates/compiler/kobo-sim-core/src/harness.rs");

    assert!(
        !harness.contains("harness_trace_hash = semantic_hash.clone()"),
        "full-depth harness must derive harness_trace_hash from actual harness output"
    );
    assert!(
        !harness.contains("agreement = \"matched\".to_owned()"),
        "agreement must be computed by comparing semantic and harness event streams"
    );
    assert!(
        harness.contains("Command::new") || harness.contains("std::process::Command"),
        "full-depth harness must compile/run generated Rust through a real process"
    );
}

#[test]
fn full_depth_harness_must_not_project_events_from_scenario_model() {
    let harness = read("../../crates/compiler/kobo-sim-core/src/harness.rs");

    assert!(
        !harness.contains("projected_harness_events"),
        "full-depth harness must not compile a scenario-projected event printer"
    );
    assert!(
        !harness.contains("generated_rust.contains(\"KOBO_EVENT:\")"),
        "full-depth harness must instrument generated Rust instead of accepting only pre-instrumented fixtures"
    );
    assert!(
        harness.contains("instrument_generated_rust"),
        "full-depth harness must build events from instrumented generated Rust behavior"
    );
}

#[test]
fn semantic_engine_must_not_lower_from_syn_file() {
    let lower = read("../../crates/compiler/kobo-sim-core/src/lower.rs");

    assert!(
        !lower.contains("parsed.syn_file()"),
        "semantic engine must consume compiler scenario/KIR facts, not syn_file()"
    );
    assert!(
        !lower.contains("use syn::"),
        "kobo-sim-core lowering must not depend on syn AST inspection for production-depth mode"
    );
}

#[test]
fn driver_scenario_must_be_kir_owned_not_syn_owned() {
    let scenario = read("../../crates/compiler/kobo-driver/src/scenario.rs");

    for banned in ["use syn::", "KoboFile", "syn_file()", "ExprMethodCall", "ItemFn"] {
        assert!(
            !scenario.contains(banned),
            "driver scenario extraction must consume KIR/codegen scenario facts, not `{banned}`"
        );
    }
    assert!(
        scenario.contains("scenario_programs"),
        "driver scenario extraction must select compiler-owned scenario programs"
    );
}

#[test]
fn error_policy_must_come_from_codegen_metadata_not_text_offsets() {
    let codegen = read("../../crates/compiler/kobo-driver/src/pipeline/codegen.rs");

    assert!(
        !codegen.contains("question_operator_offsets"),
        "typed/explicit error policy must not scan generated text for '?'"
    );
    assert!(
        !codegen.contains("error_sites(&artifacts.rs_source"),
        "error policy sites must be emitted by codegen metadata"
    );
    assert!(
        !codegen.contains("error_policy_sites: Vec::new()"),
        "codegen must populate real error policy sites"
    );
}

#[test]
fn error_policy_must_be_codegen_output_metadata() {
    let codegen = read("../../crates/compiler/kobo-driver/src/pipeline/codegen.rs");

    for banned in [
        "collect_error_policy_sites(&kobo_file",
        "generated_try_offsets",
        "syn::parse_file",
        "TrySiteVisitor",
    ] {
        assert!(
            !codegen.contains(banned),
            "driver codegen must consume kobo-codegen error policy metadata, not `{banned}`"
        );
    }
    assert!(
        codegen.contains("error_policy_sites"),
        "driver codegen artifacts must carry codegen-owned error policy metadata"
    );
}

#[test]
fn lsp_must_not_invent_witness_paths() {
    let lsp = read("src/commands/lsp_diagnostics.rs");

    assert!(
        !lsp.contains("fallback_semantic_diagnostics"),
        "LSP must not run hidden semantic simulation when no witness exists"
    );
    assert!(
        !lsp.contains("EngineMode::SemanticOnly"),
        "LSP artifact diagnostics must be based on real .kwit artifacts"
    );
    assert!(
        !lsp.contains("join(format!(\"{}-0.kwit\""),
        "LSP must not fabricate witness paths"
    );
}
