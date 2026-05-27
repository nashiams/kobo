use super::*;

pub(crate) fn execution_digest_json(
    run: &FullDepthRun,
    runtime_profile_hash: &str,
) -> serde_json::Value {
    serde_json::json!({
        "engine": "semantic-sim",
        "semantic_engine": run.digest.semantic_engine,
        "harness_engine": run.digest.harness_engine,
        "model_schema": run.digest.model_schema,
        "schema_version": run.digest.schema_version,
        "scenario_ir_hash": run.digest.scenario_ir_hash,
        "operation_count": run.digest.operation_count,
        "event_hash": run.digest.semantic_trace_hash,
        "semantic_trace_hash": run.digest.semantic_trace_hash,
        "harness_trace_hash": run.digest.harness_trace_hash,
        "fuzz_driver_trace_hash": run.digest.fuzz_driver_trace_hash,
        "fuzz_driver_event_count": run.digest.fuzz_driver_event_count,
        "agreement": run.digest.agreement,
        "generated_rust_hash": run.digest.generated_rust_hash,
        "harness_manifest_hash": run.digest.harness_manifest_hash,
        "harness_exit_code": run.digest.harness_exit_code,
        "harness_event_count": run.digest.harness_event_count,
        "runtime_profile_hash": runtime_profile_hash,
    })
}

pub(crate) fn backend_version_json(backend: &str) -> serde_json::Value {
    let capability = kobo_sim_core::backend::capabilities()
        .iter()
        .find(|capability| capability.name == backend);
    serde_json::json!({
        "backend": backend,
        "adapter_source": "kobo-sim-core",
        "adapter_version": env!("CARGO_PKG_VERSION"),
        "integration_level": capability.map(|capability| capability.integration_level),
        "scenario_execution": capability.map(|capability| capability.scenario_execution),
    })
}

pub(crate) fn checkpoint_replay_json(enabled: bool, run: &FullDepthRun) -> serde_json::Value {
    let checkpoint_path = run
        .harness_manifest
        .as_ref()
        .and_then(|manifest| manifest.checkpoint_path.as_deref());
    let checkpoint_artifact_hash = run
        .harness_manifest
        .as_ref()
        .and_then(|manifest| manifest.checkpoint_artifact_hash.as_deref());
    let artifact_validated =
        enabled && checkpoint_path.is_some() && checkpoint_artifact_hash.is_some();
    serde_json::json!({
        "enabled": artifact_validated,
        "semantic_trace_hash": if artifact_validated {
            Some(run.digest.semantic_trace_hash.as_str())
        } else {
            None
        },
        "harness_trace_hash": if artifact_validated {
            Some(run.digest.harness_trace_hash.as_str())
        } else {
            None
        },
        "event_count": if artifact_validated {
            Some(run.events.len())
        } else {
            None
        },
        "checkpoint_path": if artifact_validated {
            checkpoint_path
        } else {
            None
        },
        "checkpoint_artifact_hash": if artifact_validated {
            checkpoint_artifact_hash
        } else {
            None
        },
        "artifact_validation": if artifact_validated {
            Some("loom-checkpoint-hash")
        } else {
            None
        },
        "source": if artifact_validated {
            Some("loom-builder-checkpoint")
        } else {
            None
        },
    })
}

pub(crate) fn coverage_json(run: &FullDepthRun) -> serde_json::Value {
    let covered = run
        .obligations
        .iter()
        .filter(|obligation| obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .collect::<Vec<_>>();
    let uncovered = run
        .obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .collect::<Vec<_>>();
    serde_json::json!({
        "unsupported_constructs": run.coverage.unsupported_constructs,
        "reason": run.coverage.reason,
        "covered": covered,
        "uncovered": uncovered,
        "escaped": escaped_obligations(run),
        "suppressed": [],
        "boundary_owned": run.opaque_boundaries,
        "unknown": [],
    })
}

pub(crate) fn scenario_coverage_json(run: &FullDepthRun) -> serde_json::Value {
    coverage_json(run)
}

pub(crate) fn sim_config_json(config: &kobo_driver::KoboConfig) -> serde_json::Value {
    let profiles = config
        .sim
        .profiles
        .iter()
        .map(|(name, profile)| {
            (
                name.clone(),
                serde_json::json!({
                    "schedule_budget": profile.schedule_budget,
                    "seed_count": profile.seed_count,
                    "shrink": profile.shrink.as_deref(),
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let backends = config
        .sim
        .backends
        .iter()
        .map(|(name, backend)| {
            (
                name.clone(),
                serde_json::json!({
                    "enabled": backend.enabled,
                    "scheduler": backend.scheduler.as_deref(),
                    "replay_token": backend.replay_token.as_deref(),
                    "max_branches": backend.max_branches,
                    "checkpoint_replay": backend.checkpoint_replay,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    serde_json::json!({
        "default_profile": config.sim.default_profile.as_str(),
        "show_backend_choices": config.sim.show_backend_choices,
        "profiles": profiles,
        "backends": backends,
    })
}
