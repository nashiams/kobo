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
