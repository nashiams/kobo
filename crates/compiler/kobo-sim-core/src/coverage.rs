use kobo_errors::KErrorCode;

use crate::core::{ReplayGuarantee, ScenarioCoverage};

pub fn replay_guarantee_for(
    coverage: &ScenarioCoverage,
    agreement: &str,
    failure_code: Option<KErrorCode>,
) -> ReplayGuarantee {
    if matches!(failure_code, Some(KErrorCode::K0102 | KErrorCode::K0103)) {
        return ReplayGuarantee::NotReplayable;
    }
    if !replay_blocking_unsupported_constructs(coverage).is_empty() {
        return ReplayGuarantee::Partial;
    }
    if agreement == "matched" || agreement == "semantic-only" {
        ReplayGuarantee::Exact
    } else {
        ReplayGuarantee::NotReplayable
    }
}

pub fn replay_blocking_unsupported_constructs(coverage: &ScenarioCoverage) -> Vec<String> {
    coverage
        .unsupported_constructs
        .iter()
        .filter(|construct| construct.as_str() != "lifecycle_method_await_initializer")
        .cloned()
        .collect()
}
