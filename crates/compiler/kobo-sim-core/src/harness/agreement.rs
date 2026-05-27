use kobo_errors::KErrorCode;

use crate::core::{EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure};
use crate::error::{Result, SimCoreError};

use super::run_generated_harness;
use kobo_ir::ScenarioProgram;

enum TraceAgreement {
    Matched,
    SemanticOnly,
    Diverged,
    CoverageIncomplete,
    BoundaryPartial,
}

pub fn check_harness_agreement(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &crate::core::ScenarioOptions,
    mut semantic: FullDepthRun,
    _mode: EngineMode,
) -> Result<FullDepthRun> {
    if semantic.failure.as_ref().is_some_and(|failure| {
        matches!(
            failure.code,
            KErrorCode::K0102 | KErrorCode::K0103 | KErrorCode::K0105 | KErrorCode::K0107
        )
    }) {
        return Ok(semantic);
    }

    let replay_blocking_constructs =
        crate::coverage::replay_blocking_unsupported_constructs(&semantic.coverage);
    if !replay_blocking_constructs.is_empty() {
        semantic.digest.agreement = agreement_label(TraceAgreement::CoverageIncomplete);
        semantic.replay_guarantee = ReplayGuarantee::Partial;
        semantic.failure = Some(ScenarioFailure {
            code: KErrorCode::K0116,
            message: format!(
                "scenario coverage incomplete; exact replay is not allowed for {}",
                replay_blocking_constructs.join(", ")
            ),
            primary_start: 0,
            primary_end: 1,
            events: Vec::new(),
        });
        return Ok(semantic);
    }
    if !semantic.coverage.unsupported_constructs.is_empty() {
        semantic.digest.agreement = agreement_label(TraceAgreement::SemanticOnly);
        return Ok(semantic);
    }

    if !semantic.opaque_boundaries.is_empty() {
        semantic.digest.agreement = agreement_label(TraceAgreement::BoundaryPartial);
        semantic.replay_guarantee = ReplayGuarantee::Partial;
        return Ok(semantic);
    }

    let harness = run_generated_harness(program, generated_rust, options)?;
    let comparable_harness_events = comparable_harness_events(&harness.events);
    if events_match_with_harness_recording(&semantic.events, &comparable_harness_events) {
        merge_harness_recordings(&mut semantic, &comparable_harness_events);
        if comparable_harness_events.len() != harness.events.len() {
            semantic.events = harness.events.clone();
        }
        semantic.digest.semantic_trace_hash = crate::digest::events_hash(&semantic.events);
    }
    let mut harness_hash = crate::digest::events_hash(&harness.events);
    let manifest_json = serde_json::to_string(&harness.manifest)
        .map_err(|source| SimCoreError::json("serialize harness manifest", source))?;
    let backend_execution = crate::backend::execute_profile(
        &semantic.profile,
        options.seed,
        &semantic.digest.semantic_trace_hash,
        &harness_hash,
        &harness.manifest.generated_rust_hash,
        &harness.events,
    );

    semantic.digest.harness_engine = harness.engine.clone();
    if let Some(execution) = backend_execution {
        harness_hash =
            crate::digest::stable_hash(&format!("{}:{}", harness_hash, execution.token_material));
    }
    semantic.digest.generated_rust_hash = Some(harness.manifest.generated_rust_hash.clone());
    semantic.digest.harness_manifest_hash = Some(crate::digest::stable_hash(&manifest_json));
    semantic.digest.harness_exit_code = Some(harness.manifest.exit_code);
    semantic.digest.harness_event_count = harness.manifest.event_count;
    semantic.digest.harness_trace_hash = harness_hash;
    semantic.harness_manifest = Some(harness.manifest.clone());

    if semantic.events == harness.events {
        semantic.digest.agreement = agreement_label(TraceAgreement::Matched);
        if semantic.opaque_boundaries.is_empty()
            && !matches!(semantic.replay_guarantee, ReplayGuarantee::NotReplayable)
        {
            semantic.replay_guarantee = ReplayGuarantee::Exact;
        }
        return Ok(semantic);
    }

    semantic.digest.agreement = agreement_label(TraceAgreement::Diverged);
    semantic.replay_guarantee = ReplayGuarantee::NotReplayable;
    semantic.events.extend(harness.events.clone());
    semantic.failure.get_or_insert_with(|| ScenarioFailure {
        code: KErrorCode::K0117,
        message: "semantic trace and generated harness trace diverged".to_owned(),
        primary_start: 0,
        primary_end: 1,
        events: harness.events,
    });
    Ok(semantic)
}

fn events_match_with_harness_recording(
    semantic_events: &[ScenarioEvent],
    harness_events: &[ScenarioEvent],
) -> bool {
    semantic_events.len() == harness_events.len()
        && semantic_events
            .iter()
            .zip(harness_events)
            .all(|(semantic, harness)| {
                semantic.kind == harness.kind
                    && semantic.label == harness.label
                    && semantic.value == harness.value
                    && (semantic.io == harness.io
                        || (semantic.kind == "boundary-record"
                            && semantic.io.is_none()
                            && harness.io.is_some()))
            })
}

fn comparable_harness_events(events: &[ScenarioEvent]) -> Vec<ScenarioEvent> {
    events
        .iter()
        .filter(|event| event.kind != "service-scheduler-hook")
        .cloned()
        .collect()
}

fn merge_harness_recordings(run: &mut FullDepthRun, harness_events: &[ScenarioEvent]) {
    for (semantic, harness) in run.events.iter_mut().zip(harness_events) {
        if semantic.kind == "boundary-record" && semantic.io.is_none() {
            semantic.io = harness.io.clone();
        }
    }
    for decision in &mut run.boundary_decisions {
        if decision.policy.as_str() != "record" {
            continue;
        }
        let expected_label = format!(
            "{}@{}..{}",
            decision
                .call_path
                .as_deref()
                .unwrap_or(decision.crate_name.as_str()),
            decision.span_start,
            decision.span_end
        );
        decision.recorded_io = run
            .events
            .iter()
            .find(|event| {
                event.kind == "boundary-record"
                    && event.label.as_deref() == Some(expected_label.as_str())
            })
            .and_then(|event| event.io.clone());
    }
}

fn agreement_label(agreement: TraceAgreement) -> String {
    match agreement {
        TraceAgreement::Matched => String::from("matched"),
        TraceAgreement::SemanticOnly => String::from("semantic-only"),
        TraceAgreement::Diverged => String::from("diverged"),
        TraceAgreement::CoverageIncomplete => String::from("coverage-incomplete"),
        TraceAgreement::BoundaryPartial => String::from("partial-boundary"),
    }
}
