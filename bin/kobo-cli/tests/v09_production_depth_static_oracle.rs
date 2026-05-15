use std::fs;
use std::path::Path;

#[test]
fn production_depth_executor_does_not_call_legacy_line_scanner() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sim_model = fs::read_to_string(manifest_dir.join("src/commands/sim_model.rs"))
        .expect("sim_model.rs should be readable");
    let sim_semantic = fs::read_to_string(manifest_dir.join("src/commands/sim_semantic.rs"))
        .expect("sim_semantic.rs should be readable");
    let production_executor = format!("{sim_model}\n{sim_semantic}");

    for banned in [
        "line_infos(&scenario.body)",
        "strip_line_comment",
        "parse_method_call",
        "parse_move_binding",
        "line.find(\"SystemTime::now\")",
        "line.contains(\"ward.time\")",
    ] {
        assert!(
            !production_executor.contains(banned),
            "production-depth simulation must not call legacy source-line scanner token `{banned}`"
        );
    }

    assert!(
        production_executor.contains("scenario_program_from_document"),
        "sim quick must route behavior extraction through the semantic operation graph"
    );
    assert!(
        production_executor.contains("semantic-sim"),
        "exact witnesses must name the semantic execution engine"
    );
}

#[test]
fn typed_error_policy_does_not_rewrite_by_last_question_mark_per_line() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let build_rs = fs::read_to_string(manifest_dir.join("src/commands/build.rs"))
        .expect("build.rs should be readable");

    for banned in ["line.rfind('?')", "line.contains('?')"] {
        assert!(
            !build_rs.contains(banned),
            "typed error policy must not classify generated Rust by source-line token `{banned}`"
        );
    }
    assert!(
        build_rs.contains("question_operator_offsets"),
        "typed error policy must discover syntactic question operators before rewriting"
    );
}
