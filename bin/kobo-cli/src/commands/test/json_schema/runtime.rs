use super::*;

pub(crate) fn service_runtime_json(
    evidence: &kobo_codegen::RuntimeEvidence,
    run: &FullDepthRun,
) -> serde_json::Value {
    serde_json::json!({
        "evidence_source": "codegen-lowering",
        "services": evidence.services
            .iter()
            .map(|service| service_runtime_service_json(service, run))
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn runtime_profile_json(
    config: &kobo_driver::KoboConfig,
    sim_profile: &str,
    seed: u64,
    run: &FullDepthRun,
) -> serde_json::Value {
    let profile = &config.runtime_profile;
    serde_json::json!({
        "service": {
            "buffer": profile.service_buffer,
            "backpressure": profile.service_backpressure,
        },
        "scenario": {
            "scheduler": profile.scheduler,
            "sim_profile": sim_profile,
            "backend_profile": run.profile,
            "seed": seed,
            "event_budget": profile.scenario_event_budget,
        },
        "record": {
            "default": profile.record,
        },
        "activity": {
            "default": profile.activity,
        },
        "runtime": {
            "cancellation": profile.cancellation,
        },
    })
}

pub(crate) fn service_runtime_service_json(
    service: &kobo_codegen::ServiceRuntimeEvidence,
    run: &FullDepthRun,
) -> serde_json::Value {
    serde_json::json!({
        "name": &service.name,
        "buffer": service.buffer,
        "source_line": service.source_line,
        "backpressure": &service.backpressure,
        "dispatch_loop": service.dispatch_loop,
        "client_api": service.client_api,
        "scenario_hooks": service.scenario_hooks,
        "hook_events": &service.hook_events,
        "runtime_hook_events": service_runtime_hook_events_json(&service.name, run),
        "methods": service.methods
            .iter()
            .map(service_runtime_method_json)
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn service_runtime_hook_events_json(
    service_name: &str,
    run: &FullDepthRun,
) -> serde_json::Value {
    let events = run
        .harness_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .service_hook_events
                .iter()
                .filter(|event| event.service == service_name)
                .map(|event| {
                    serde_json::json!({
                        "phase": &event.phase,
                        "method": &event.method,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::Value::Array(events)
}

pub(crate) fn service_runtime_method_json(
    method: &kobo_codegen::ServiceRuntimeMethodEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "name": &method.name,
        "variant": &method.variant,
    })
}

pub(crate) fn handler_lifecycle_json(
    evidence: &kobo_codegen::RuntimeEvidence,
    run: &FullDepthRun,
) -> serde_json::Value {
    serde_json::json!({
        "evidence_source": "codegen-lowering",
        "handlers": evidence.handlers
            .iter()
            .map(handler_lifecycle_handler_json)
            .collect::<Vec<_>>(),
        "scenario_cases": handler_scenario_cases(run),
    })
}

pub(crate) fn handler_lifecycle_handler_json(
    handler: &kobo_codegen::HandlerLifecycleEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "name": &handler.name,
        "source_line": handler.source_line,
        "cleanup_hook": &handler.cleanup_hook,
        "must_call": {
            "kind": "handler_token",
            "terminal_actions": &handler.terminal_actions,
            "required": true,
        },
        "boundaries": {
            "tracing": &handler.tracing_boundary,
            "metrics": &handler.metrics_boundary,
            "cleanup": &handler.cleanup_boundary,
            "cancel_cleanup": &handler.cancel_cleanup,
        },
        "terminal_evidence_source": &handler.terminal_evidence_source,
    })
}

pub(crate) fn parallel_lowering_json(
    evidence: &kobo_codegen::RuntimeEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "evidence_source": "codegen-lowering",
        "loops": evidence.parallel_loops
            .iter()
            .map(parallel_loop_json)
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn parallel_loop_json(
    loop_evidence: &kobo_codegen::ParallelLoopEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "source_line": loop_evidence.source_line,
        "lowering": &loop_evidence.lowering,
        "policy": &loop_evidence.policy,
        "analysis_gate": &loop_evidence.analysis_gate,
        "proof": &loop_evidence.proof,
        "iterator": &loop_evidence.iterator,
        "captured_bindings": &loop_evidence.captured_bindings,
        "safety_checks": &loop_evidence.safety_checks,
    })
}

pub(crate) fn task_local_zones_json(evidence: &kobo_codegen::RuntimeEvidence) -> serde_json::Value {
    serde_json::json!({
        "evidence_source": "codegen-lowering",
        "zones": evidence.task_local_zones
            .iter()
            .map(task_local_zone_json)
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn task_local_zone_json(zone: &kobo_codegen::TaskLocalEvidence) -> serde_json::Value {
    serde_json::json!({
        "source_line": zone.source_line,
        "strategy": &zone.strategy,
        "proof": &zone.proof,
        "captured_bindings": &zone.captured_bindings,
        "safety_checks": &zone.safety_checks,
    })
}

pub(crate) fn handler_scenario_cases(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut cases = Vec::new();
    if run
        .events
        .iter()
        .any(|event| event.kind == "network-dropped" || event.kind == "network-drop-message")
    {
        cases.push(serde_json::json!({
            "kind": "disconnect",
            "source": "network-model",
            "event": "network-drop-message",
        }));
    }
    if run
        .events
        .iter()
        .any(|event| event.kind == "failure-injection-cancel")
    {
        cases.push(serde_json::json!({
            "kind": "cancellation",
            "source": "scheduler",
            "event": "failure-injection-cancel",
        }));
    }
    cases
}
