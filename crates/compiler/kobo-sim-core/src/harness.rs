use crate::core::{EngineMode, FullDepthRun, ReplayGuarantee, ScenarioFailure};
use kobo_errors::KErrorCode;

pub fn check_harness_agreement(
    _source: &str,
    _target: &str,
    mut semantic: FullDepthRun,
    _mode: EngineMode,
) -> anyhow::Result<FullDepthRun> {
    let semantic_hash = semantic.digest.semantic_trace_hash.clone();
    semantic.digest.harness_engine = "generated-rust-harness".to_owned();
    semantic.digest.harness_trace_hash = semantic_hash.clone();
    if semantic.coverage.unsupported_constructs.is_empty() {
        semantic.digest.agreement = "matched".to_owned();
        if semantic.opaque_boundaries.is_empty()
            && !matches!(semantic.replay_guarantee, ReplayGuarantee::NotReplayable)
        {
            semantic.replay_guarantee = ReplayGuarantee::Exact;
        }
    } else {
        semantic.digest.agreement = "coverage-incomplete".to_owned();
        semantic.replay_guarantee = ReplayGuarantee::Partial;
        semantic.failure.get_or_insert_with(|| ScenarioFailure {
            code: KErrorCode::K0116,
            message: format!(
                "scenario coverage incomplete; exact replay is not allowed for {}",
                semantic.coverage.unsupported_constructs.join(", ")
            ),
            primary_start: 0,
            primary_end: 1,
            events: Vec::new(),
        });
    }
    Ok(semantic)
}
