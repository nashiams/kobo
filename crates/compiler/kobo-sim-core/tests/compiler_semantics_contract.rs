use kobo_ir::{
    FileId, KoboSpan, ScenarioCoverageFacts, ScenarioModeledBoundary, ScenarioOp, ScenarioOpKind,
    ScenarioProgram,
};
use kobo_sim_core::{
    run_full_depth, run_semantics_from_program, EngineMode, ReplayGuarantee, ScenarioOptions,
};

#[test]
fn compiler_semantics_tracks_scenario_operations_and_obligation() {
    let program = ScenarioProgram {
        file_id: FileId(0),
        target: "scenario".to_owned(),
        source_hash: "hash".to_owned(),
        operations: vec![
            ScenarioOp {
                span: KoboSpan::new(4, 12, FileId(0)),
                kind: ScenarioOpKind::CreateObligation {
                    binding: "delivery".to_owned(),
                    type_name: "Delivery".to_owned(),
                    actions: vec!["ack".to_owned(), "nack".to_owned()],
                    template: None,
                },
            },
            ScenarioOp {
                span: KoboSpan::new(20, 30, FileId(0)),
                kind: ScenarioOpKind::ModeledEffect {
                    boundary: ScenarioModeledBoundary::WardTask,
                },
            },
            ScenarioOp {
                span: KoboSpan::new(31, 45, FileId(0)),
                kind: ScenarioOpKind::Discharge {
                    binding: "delivery".to_owned(),
                    action: "ack".to_owned(),
                },
            },
        ],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    let run = run_semantics_from_program(&program, &ScenarioOptions::default())
        .expect("compiler semantics should run");

    assert_eq!(run.replay_guarantee, ReplayGuarantee::Exact);
    assert!(
        run.failure.is_none(),
        "obligation should be discharged through scenario operations"
    );
    assert!(run.coverage.unsupported_constructs.is_empty());
}

#[test]
fn compiler_semantics_reports_coverage_gap_for_unmodeled_construct() {
    let program = ScenarioProgram {
        file_id: FileId(0),
        target: "scenario".to_owned(),
        source_hash: "hash".to_owned(),
        operations: Vec::new(),
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts {
            unsupported_constructs: vec!["tokio::select!".to_owned()],
            ..ScenarioCoverageFacts::default()
        },
    };

    let run = run_semantics_from_program(&program, &ScenarioOptions::default())
        .expect("coverage gap should be represented");

    assert_eq!(run.replay_guarantee, ReplayGuarantee::Partial);
    assert!(
        run.coverage
            .unsupported_constructs
            .iter()
            .any(|item| item.contains("tokio::select")),
        "coverage gap should name tokio::select"
    );
}

#[test]
fn source_level_full_depth_replay_compiles_source_without_driver_program() {
    let source = r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "sync")]
fn direct_source_scenario() {
    let delivery = Delivery {};
    delivery.ack();
}
"#;

    let run = run_full_depth(
        source,
        "direct_source_scenario",
        EngineMode::Both,
        ScenarioOptions {
            profile: "sync".to_owned(),
            ..ScenarioOptions::default()
        },
    )
    .expect("source-level full-depth replay should compile source directly");

    assert_eq!(run.target, "direct_source_scenario");
    assert_eq!(run.replay_guarantee, ReplayGuarantee::Exact);
    assert_eq!(run.digest.semantic_engine, "driver-kir-scenario");
    assert_eq!(run.digest.harness_engine, "generated-rust-loom-process");
}
