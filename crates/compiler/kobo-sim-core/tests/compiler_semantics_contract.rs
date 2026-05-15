use kobo_sim_core::{run_compiler_semantics, ReplayGuarantee};

#[test]
fn compiler_semantics_tracks_match_helper_and_obligation() {
    let source = r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

fn finish(delivery: Delivery, should_ack: bool) {
    match should_ack {
        true => delivery.ack(),
        false => delivery.nack(),
    }
}

#[kobo::scenario(profile = "async")]
fn scenario() {
    let delivery = Delivery {};
    finish(delivery, true);
}
"#;
    let run = run_compiler_semantics(source, "scenario").expect("compiler semantics should run");
    assert_eq!(run.replay_guarantee, ReplayGuarantee::Exact);
    assert!(run.failure.is_none(), "obligation should be discharged through helper match");
    assert!(run.coverage.unsupported_constructs.is_empty());
}

#[test]
fn compiler_semantics_reports_coverage_gap_for_unlowered_macro() {
    let source = r#"
#[kobo::scenario(profile = "async")]
async fn scenario() {
    tokio::select! {
        _ = async { ward.task(); } => {}
        _ = async { ward.time.now(); } => {}
    }
}
"#;
    let run = run_compiler_semantics(source, "scenario").expect("coverage gap should be represented");
    assert_eq!(run.replay_guarantee, ReplayGuarantee::Partial);
    assert!(
        run.coverage
            .unsupported_constructs
            .iter()
            .any(|item| item.contains("tokio::select")),
        "coverage gap should name tokio::select"
    );
}
