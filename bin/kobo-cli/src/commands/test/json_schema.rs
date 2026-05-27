use super::failure::scenario_failure_has_event;
use super::*;

pub(super) fn service_runtime_json(
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

pub(super) fn runtime_profile_json(
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

pub(super) fn service_runtime_service_json(
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

pub(super) fn service_runtime_hook_events_json(
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

pub(super) fn service_runtime_method_json(
    method: &kobo_codegen::ServiceRuntimeMethodEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "name": &method.name,
        "variant": &method.variant,
    })
}

pub(super) fn handler_lifecycle_json(
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

pub(super) fn handler_lifecycle_handler_json(
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

pub(super) fn parallel_lowering_json(
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

pub(super) fn parallel_loop_json(
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

pub(super) fn task_local_zones_json(evidence: &kobo_codegen::RuntimeEvidence) -> serde_json::Value {
    serde_json::json!({
        "evidence_source": "codegen-lowering",
        "zones": evidence.task_local_zones
            .iter()
            .map(task_local_zone_json)
            .collect::<Vec<_>>(),
    })
}

pub(super) fn task_local_zone_json(zone: &kobo_codegen::TaskLocalEvidence) -> serde_json::Value {
    serde_json::json!({
        "source_line": zone.source_line,
        "strategy": &zone.strategy,
        "proof": &zone.proof,
        "captured_bindings": &zone.captured_bindings,
        "safety_checks": &zone.safety_checks,
    })
}

pub(super) fn handler_scenario_cases(run: &FullDepthRun) -> Vec<serde_json::Value> {
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

pub(super) fn execution_digest_json(
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

pub(super) fn backend_version_json(backend: &str) -> serde_json::Value {
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

pub(super) fn checkpoint_replay_json(enabled: bool, run: &FullDepthRun) -> serde_json::Value {
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

pub(super) fn coverage_json(run: &FullDepthRun) -> serde_json::Value {
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

pub(super) fn scenario_coverage_json(run: &FullDepthRun) -> serde_json::Value {
    coverage_json(run)
}

pub(super) fn sim_config_json(config: &kobo_driver::KoboConfig) -> serde_json::Value {
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

pub(super) fn scheduler_json(
    sim_profile: &str,
    seed: u64,
    run: &FullDepthRun,
    runtime_scheduler: Option<&str>,
) -> serde_json::Value {
    let profile_strategy = match sim_profile {
        "quick" => "small-random",
        "deep" => "pct-random-bounded",
        "replay" => "witness-event-stream",
        "exhaustive" => "tiny-ward-exhaustive",
        _ => "unknown",
    };
    let strategy = runtime_scheduler
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(profile_strategy);
    let seeds = scheduler_seed_cases(run, seed);
    serde_json::json!({
        "profile": sim_profile,
        "strategy": strategy,
        "seed": seed,
        "seed_count": seeds.len(),
        "seeds": seeds,
        "event_budget": run
            .events
            .iter()
            .find(|event| event.kind == "scheduler-portfolio")
            .and_then(|event| event.value),
        "cancellation": scheduler_cancellation_json(run),
    })
}

pub(super) fn scheduler_seed_cases(run: &FullDepthRun, fallback_seed: u64) -> Vec<u64> {
    let seeds = run
        .events
        .iter()
        .filter(|event| event.kind == "scheduler-seed-case")
        .filter_map(|event| event.value)
        .collect::<Vec<_>>();
    if seeds.is_empty() {
        vec![fallback_seed]
    } else {
        seeds
    }
}

pub(super) fn scheduler_cancellation_json(run: &FullDepthRun) -> serde_json::Value {
    let events = scheduler_cancellation_events(run);
    if events.is_empty() {
        return serde_json::json!({
            "mode": "none",
            "token_source": null,
            "events": [],
        });
    }
    serde_json::json!({
        "mode": "explicit-scheduler-history",
        "token_source": "kobo.scheduler.cancel",
        "events": events,
    })
}

pub(super) fn scheduler_cancellation_events(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.events
        .iter()
        .filter(|event| is_scheduler_cancellation_event(&event.kind))
        .map(|event| {
            serde_json::json!({
                "kind": event.kind.clone(),
                "label": event.label.clone(),
                "value": event.value,
            })
        })
        .collect()
}

pub(super) fn is_scheduler_cancellation_event(kind: &str) -> bool {
    matches!(
        kind,
        "scheduler-cancel-path" | "scheduler-future-dropped" | "failure-injection-cancel"
    )
}

pub(super) fn modeled_boundaries_json(run: &FullDepthRun) -> Vec<&'static str> {
    run.modeled_boundaries
        .iter()
        .map(|boundary| boundary.as_str())
        .collect()
}

pub(super) fn boundary_policies_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut policies = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
            })
        })
        .collect::<Vec<_>>();

    for boundary in modeled_boundaries_json(run) {
        policies.push(serde_json::json!({
            "boundary": boundary,
            "policy": "model",
            "reason": "modeled compiler facade",
        }));
    }
    policies
}

pub(super) fn ecosystem_boundaries_json(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            let evidence = boundary_evidence_for_policy(decision, run);
            serde_json::json!({
                "crate": decision.crate_name,
                "call_path": decision.call_path,
                "call_arguments": decision.call_arguments,
                "return_type": decision.return_type,
                "call_shape": decision.call_shape.as_str(),
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "evidence": evidence,
                "capture": boundary_capture_json(decision, run),
                "activity_metadata": activity_metadata_for_boundary(file, config, decision),
                "source_span": {
                    "start": decision.span_start,
                    "end": decision.span_end,
                },
                "declaration": declaration_metadata_for_boundary(file, config, &decision.crate_name, decision.policy.as_str()),
                "adapter": config.ecosystem_policy.adapter_for(&decision.crate_name).map(|adapter| serde_json::json!({
                    "package": adapter.package,
                    "version": adapter.version,
                    "confidence": adapter.confidence,
                    "source": adapter.source,
                    "registry": adapter.registry,
                    "checksum": adapter.checksum,
                    "compatible_crate": adapter.compatible_crate,
                    "metadata_path": adapter.metadata_path.as_ref().map(|path| path.display().to_string()),
                    "trust_policy": adapter.trust_policy,
                    "signed_by": adapter.signed_by,
                    "validated": adapter.validated,
                    "adapter_runtime": adapter.adapter_runtime,
                    "capture": adapter.capture,
                    "reason": adapter.reason,
                })),
                "full_ecosystem_exploration": false,
            })
        })
        .collect()
}

pub(super) fn declaration_metadata_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    crate_name: &str,
    policy: &str,
) -> Option<serde_json::Value> {
    if !matches!(policy, "typed" | "activity") {
        return None;
    }
    let facts = match declarations::declaration_facts_for_config(file, crate_name, config) {
        declarations::DeclarationLookup::Valid(facts) => facts,
        declarations::DeclarationLookup::Missing | declarations::DeclarationLookup::Invalid(_) => {
            return None;
        }
    };
    Some(declaration_metadata_json(&facts))
}

pub(super) fn activity_metadata_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    decision: &kobo_sim_core::BoundaryDecision,
) -> Option<serde_json::Value> {
    if decision.policy.as_str() != "activity" {
        return None;
    }
    let facts = match declarations::declaration_facts_for_config(file, &decision.crate_name, config)
    {
        declarations::DeclarationLookup::Valid(facts) => facts,
        declarations::DeclarationLookup::Missing | declarations::DeclarationLookup::Invalid(_) => {
            return None;
        }
    };
    let activity = declarations::activity_fact_for_call(&facts, decision.call_path.as_deref())?;
    Some(serde_json::json!({
        "path": activity.path.clone(),
        "retry": activity.retry.clone(),
        "idempotency": activity.idempotency.clone(),
        "result": activity.result.clone(),
        "compensation": activity.compensation.clone(),
        "declaration_hash": facts.hash.clone(),
        "declaration_version": facts.version.clone(),
    }))
}

pub(super) fn declarations_json(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .filter_map(|decision| {
            if decision.policy.as_str() != "typed" {
                return None;
            }
            let facts = match declarations::declaration_facts_for_config(
                file,
                &decision.crate_name,
                config,
            ) {
                declarations::DeclarationLookup::Valid(facts) => facts,
                declarations::DeclarationLookup::Missing
                | declarations::DeclarationLookup::Invalid(_) => return None,
            };
            Some(serde_json::json!({
                "crate": decision.crate_name,
                "path": facts.path.display().to_string(),
                "version": facts.version.clone(),
                "schema_version": facts.schema_version,
                "hash": facts.hash.clone(),
                "declaration_version": facts.version.clone(),
                "declaration_hash": facts.hash.clone(),
            }))
        })
        .collect()
}

pub(super) fn declaration_metadata_json(
    facts: &declarations::DeclarationFacts,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "path": facts.path.display().to_string(),
        "version": facts.version.clone(),
        "schema_version": facts.schema_version,
        "hash": facts.hash.clone(),
    });
    if let Some(package) = facts.metadata_package.as_ref() {
        value
            .as_object_mut()
            .expect("declaration metadata json should be an object")
            .insert(
                "metadata_package".to_owned(),
                serde_json::json!({
                    "package": package.package.clone(),
                    "version": package.version.clone(),
                    "path": package.path.display().to_string(),
                    "source": package.source.clone(),
                    "registry": package.registry.clone(),
                    "checksum": package.checksum.clone(),
                    "signed_by": package.signed_by.clone(),
                    "validated": package.validated,
                }),
            );
    }
    value
}

pub(super) fn summary_usage_json(
    config: &kobo_driver::KoboConfig,
    program: &ScenarioProgram,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut summaries = Vec::new();
    summaries.push(formal_core::summary_json(program));
    for summary in &config.ecosystem_policy.summaries {
        let valid = summary_validation::load_valid_summary(summary)?;
        let parsed = valid.value;
        let obligations = parsed
            .get("obligations")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let functions = parsed
            .get("functions")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let solver_metadata = parsed
            .get("solver_metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"engine": "unknown", "outcome": "missing"}));
        summaries.push(serde_json::json!({
            "crate": summary.crate_name,
            "path": summary.path.display().to_string(),
            "summary_hash": valid.hash,
            "schema_version": valid.schema_version,
            "solver_metadata": solver_metadata,
            "obligation_count": obligations.len(),
            "function_count": functions.len(),
            "obligations": obligations,
            "functions": functions,
        }));
    }
    Ok(summaries)
}

pub(super) fn boundary_evidence_for_policy(
    decision: &kobo_sim_core::BoundaryDecision,
    run: &FullDepthRun,
) -> &'static str {
    let expected_label = boundary_event_label(decision);
    match decision.policy.as_str() {
        "record"
            if run.events.iter().any(|event| {
                event.kind == "boundary-record"
                    && event.label.as_deref() == Some(expected_label.as_str())
            }) =>
        {
            "recorded-event"
        }
        "activity"
            if run.events.iter().any(|event| {
                event.kind == "boundary-activity"
                    && event.label.as_deref() == Some(expected_label.as_str())
            }) =>
        {
            "activity-result"
        }
        "model" => "modeled-facade",
        "typed" => "declaration",
        "stub" => "scenario-stub",
        "outside" => "outside-assumption",
        "opaque" | "debt" => "assumption",
        _ => "unverified",
    }
}

pub(super) fn boundary_capture_json(
    decision: &kobo_sim_core::BoundaryDecision,
    run: &FullDepthRun,
) -> Option<serde_json::Value> {
    let expected_label = boundary_event_label(decision);
    let event = run.events.iter().find(|event| {
        matches!(
            event.kind.as_str(),
            "boundary-record" | "boundary-activity" | "boundary-model" | "boundary-stub"
        ) && event.label.as_deref() == Some(expected_label.as_str())
    })?;
    let capture_source = if run.harness_manifest.is_some() {
        "facade-call-capture"
    } else {
        "semantic-boundary-capture"
    };
    Some(serde_json::json!({
        "mode": "boundary-call-capture",
        "capture_source": capture_source,
        "event_kind": event.kind.clone(),
        "event_label": event.label.clone(),
        "event_value": event.value,
        "io_capture": boundary_decision_io_capture_json(decision),
        "call_path": decision.call_path.clone(),
        "call_arguments": decision.call_arguments.clone(),
        "return_type": decision.return_type.clone(),
        "call_shape": decision.call_shape.as_str(),
        "source_span": {
            "start": decision.span_start,
            "end": decision.span_end,
        },
        "external_internals_replayed": false,
    }))
}

pub(super) fn boundary_decision_io_capture_json(
    decision: &kobo_sim_core::BoundaryDecision,
) -> Option<serde_json::Value> {
    if !matches!(decision.policy.as_str(), "record" | "activity") {
        return None;
    }
    let capture = decision.recorded_io.as_ref()?;
    Some(boundary_io_capture_json(capture))
}

pub(super) fn boundary_io_payload_json(
    payload: &kobo_sim_core::BoundaryIoPayload,
) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    for field in &payload.fields {
        fields.insert(
            field.key.clone(),
            serde_json::Value::String(field.value.clone()),
        );
    }
    serde_json::json!({
        "kind": payload.kind.clone(),
        "payload": fields,
    })
}

pub(super) fn boundary_event_label(decision: &kobo_sim_core::BoundaryDecision) -> String {
    format!(
        "{}@{}..{}",
        decision
            .call_path
            .as_deref()
            .unwrap_or(decision.crate_name.as_str()),
        decision.span_start,
        decision.span_end
    )
}

pub(super) fn obligations_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding,
                "type": obligation.type_name,
                "actions": obligation.actions,
                "discharged": obligation.is_discharged,
                "declaration_span": span_json(source_path, source, obligation.declaration_span),
                "drop_span": obligation.drop_span.map(|span| span_json(source_path, source, span)),
            })
        })
        .collect()
}

pub(super) fn obligation_events_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for obligation in &run.obligations {
        events.push(serde_json::json!({
            "state": "created",
            "binding": obligation.binding,
            "type": obligation.type_name,
        }));
        if obligation.is_discharged {
            events.push(serde_json::json!({
                "state": "discharged",
                "binding": obligation.binding,
                "actions": obligation.actions,
            }));
        } else {
            events.push(serde_json::json!({
                "state": "leaked",
                "binding": obligation.binding,
                "failure_mode": unresolved_failure_mode(obligation),
            }));
        }
    }
    for event in &run.events {
        if event.kind == "obligation-transfer" {
            events.push(serde_json::json!({
                "state": "transferred",
                "binding": event.label,
            }));
        }
    }
    if run.failure.is_none() {
        events.push(serde_json::json!({
            "state": "returned",
            "binding": null,
        }));
    }
    events
}

pub(super) fn trace_event_json(index: usize, event: &ScenarioEvent) -> serde_json::Value {
    serde_json::json!({
        "id": index,
        "kind": event.kind.clone(),
        "label": event.label.clone(),
        "value": event.value,
    })
}

pub(crate) fn flagship_demo_json(
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> serde_json::Value {
    let facts = FlagshipDemoFacts::from_program(program);
    if is_durable_queue_demo(&facts, run) {
        return serde_json::json!({
            "name": "durable_queue",
            "history": if facts.has_crash_storage_action() || has_event_kind(run, "lost-message") {
                "crash_after_ack"
            } else {
                "happy_path"
            },
            "replayable_kwit": run.replay_guarantee == ReplayGuarantee::Exact,
            "capabilities": {
                "crash_histories": facts.has_crash_storage_action() || has_event_kind(run, "lost-message"),
                "storage_facade": facts.has_storage_facade(),
                "ack_nack_requeue_lifecycle": has_lifecycle_actions(run, &["ack", "nack", "requeue"]),
                "clean_rust_output": harness_built_cleanly(run),
                "ports_recordings_debt": has_boundary_or_debt_evidence(run),
                "trace_check": run.failure.as_ref().is_some_and(|failure| failure.code == KErrorCode::K0108),
            },
            "evidence_inputs": flagship_evidence_inputs(&facts, run),
        });
    }
    if is_async_gateway_demo(&facts, run) {
        return serde_json::json!({
            "name": "async_gateway",
            "scheduler_preset": run.profile.clone(),
            "replayable_kwit": run.replay_guarantee == ReplayGuarantee::Exact,
            "capabilities": {
                "cancellation_histories": has_event_kind(run, "failure-injection-cancel")
                    || has_event_kind(run, "scheduler-cancel-path"),
                "preemption_histories": has_event_kind(run, "failure-injection-preempt"),
                "reply_reject_cancel_lifecycle": has_lifecycle_actions(run, &["reply", "reject", "cancel"]),
                "no_orphan_tasks": run.failure.as_ref().is_some_and(|failure| {
                    failure.message.contains("no_orphan_tasks")
                        || failure.events.iter().any(|event| event.label.as_deref() == Some("no_orphan_tasks"))
                }),
                "request_token_diagnostics": run.failure.as_ref().is_some_and(|failure| {
                    failure.message.contains("reply")
                        || failure.message.contains("reject")
                        || failure.message.contains("cancel")
                }),
                "clean_rust_output": harness_built_cleanly(run),
            },
            "evidence_inputs": flagship_evidence_inputs(&facts, run),
        });
    }
    serde_json::Value::Null
}

#[derive(Clone, Debug, Default)]
pub(super) struct FlagshipDemoFacts {
    template_ids: Vec<String>,
    storage_actions: Vec<String>,
    modeled_boundaries: Vec<String>,
}

impl FlagshipDemoFacts {
    fn from_program(program: &ScenarioProgram) -> Self {
        let mut facts = Self::default();
        for operation in &program.operations {
            match &operation.kind {
                ScenarioOpKind::CreateObligation {
                    template: Some(template),
                    ..
                } => push_unique(&mut facts.template_ids, template.id.clone()),
                ScenarioOpKind::StorageEvent { action } => {
                    push_unique(&mut facts.storage_actions, action.clone());
                }
                ScenarioOpKind::ModeledEffect { boundary } => {
                    push_unique(&mut facts.modeled_boundaries, boundary.as_str().to_owned());
                }
                _ => {}
            }
        }
        facts
    }

    fn has_storage_facade(&self) -> bool {
        !self.storage_actions.is_empty()
            || self
                .modeled_boundaries
                .iter()
                .any(|boundary| boundary == "ward.storage")
    }

    fn has_crash_storage_action(&self) -> bool {
        self.storage_actions.iter().any(|action| {
            matches!(
                normalize_demo_action(action).as_str(),
                "crash" | "crash_after_write" | "crash_after_commit"
            )
        })
    }

    fn has_template(&self, template_id: &str) -> bool {
        self.template_ids
            .iter()
            .any(|candidate| candidate == template_id)
    }

    fn has_modeled_boundary(&self, boundary: &str) -> bool {
        self.modeled_boundaries
            .iter()
            .any(|candidate| candidate == boundary)
    }
}

pub(super) fn is_durable_queue_demo(facts: &FlagshipDemoFacts, run: &FullDepthRun) -> bool {
    has_lifecycle_actions(run, &["ack", "nack", "requeue"])
        && (facts.has_template("queue_delivery") || facts.has_storage_facade())
}

pub(super) fn is_async_gateway_demo(facts: &FlagshipDemoFacts, run: &FullDepthRun) -> bool {
    has_lifecycle_actions(run, &["reply", "reject", "cancel"])
        && (run.profile == "async"
            || facts.has_template("handler_reply")
            || facts.has_modeled_boundary("ward.task")
            || has_event_kind(run, "scheduler-cancel-path")
            || has_event_kind(run, "failure-injection-cancel")
            || has_event_kind(run, "failure-injection-preempt"))
}

pub(super) fn harness_built_cleanly(run: &FullDepthRun) -> bool {
    run.harness_manifest
        .as_ref()
        .is_some_and(|manifest| manifest.exit_code == 0 && !manifest.harness_rs_path.is_empty())
}

pub(super) fn has_boundary_or_debt_evidence(run: &FullDepthRun) -> bool {
    !run.boundary_decisions.is_empty() || !run.opaque_boundaries.is_empty()
}

pub(super) fn flagship_evidence_inputs(
    facts: &FlagshipDemoFacts,
    run: &FullDepthRun,
) -> serde_json::Value {
    serde_json::json!({
        "source": "compiler-scenario-program",
        "target": run.target,
        "profile": run.profile,
        "template_ids": facts.template_ids,
        "storage_actions": facts.storage_actions,
        "modeled_boundaries": facts.modeled_boundaries,
        "event_count": run.events.len(),
        "obligation_count": run.obligations.len(),
        "boundary_decision_count": run.boundary_decisions.len(),
        "harness_manifest": run.harness_manifest.as_ref().map(|manifest| {
            serde_json::json!({
                "execution_scope": manifest.execution_scope,
                "harness_rs_path": manifest.harness_rs_path,
                "exit_code": manifest.exit_code,
                "event_count": manifest.event_count,
            })
        }),
    })
}

pub(super) fn has_event_kind(run: &FullDepthRun, kind: &str) -> bool {
    run.events.iter().any(|event| event.kind == kind)
        || run
            .failure
            .as_ref()
            .is_some_and(|failure| failure.events.iter().any(|event| event.kind == kind))
}

pub(super) fn normalize_demo_action(action: &str) -> String {
    action.trim().replace('-', "_")
}

pub(super) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|candidate| candidate == &value) {
        values.push(value);
    }
}

pub(super) fn has_lifecycle_actions(run: &FullDepthRun, expected: &[&str]) -> bool {
    run.obligations.iter().any(|obligation| {
        expected
            .iter()
            .all(|action| obligation.actions.iter().any(|item| item == action))
    })
}

pub(super) fn expanded_policy_json(profile: &str) -> serde_json::Value {
    let (ownership, liveness, replay, boundaries, errors) = match profile {
        "dev" => ("record", "record", "record", "record", "ergonomic"),
        "release" => ("strict", "checked", "checked", "strict", "explicit"),
        _ => ("checked", "checked", "checked", "checked", "typed"),
    };
    serde_json::json!({
        "profile": profile,
        "ownership": ownership,
        "liveness": liveness,
        "replay": replay,
        "boundaries": boundaries,
        "errors": errors,
    })
}

pub(super) fn boundary_assumptions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut assumptions = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "replay_effect": if run.replay_guarantee == ReplayGuarantee::Exact {
                    "modeled"
                } else {
                    run.replay_guarantee.as_str()
                },
            })
        })
        .collect::<Vec<_>>();

    if run.replay_guarantee == ReplayGuarantee::NotReplayable && assumptions.is_empty() {
        let boundary = run
            .failure
            .as_ref()
            .and_then(|failure| failure.events.first())
            .and_then(|event| event.label.clone())
            .unwrap_or_else(|| "uncontrolled".to_owned());
        let reason = run
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "not replayable".to_owned());
        assumptions.push(serde_json::json!({
            "boundary": boundary,
            "policy": "debt",
            "reason": reason,
            "replay_effect": run.replay_guarantee.as_str(),
        }));
    }

    assumptions
}

pub(super) fn failure_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    primary_span: Option<String>,
) -> serde_json::Value {
    let Some(failure) = run.failure.as_ref() else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "message": failure.message,
        "mode": run.failure
            .as_ref()
            .map(|_| run_failure_mode(run))
            .unwrap_or("none"),
        "primary_span": primary_span.unwrap_or_else(|| format!("{source_path}:1:1")),
        "related_spans": related_spans_json(source_path, source, run, failure),
    })
}

pub(super) fn source_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    let mut spans = Vec::new();
    if let Some(failure) = run.failure.as_ref() {
        spans.push(span_json(
            source_path,
            source,
            (failure.primary_start, failure.primary_end),
        ));
    }
    for obligation in &run.obligations {
        spans.push(span_json(source_path, source, obligation.declaration_span));
        if let Some(drop_span) = obligation.drop_span {
            spans.push(span_json(source_path, source, drop_span));
        }
    }
    spans
}

pub(super) fn run_failure_mode(run: &FullDepthRun) -> &'static str {
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "model-trace-divergence"))
    {
        return "model_trace_divergence";
    }
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "model-obligation-divergence"))
    {
        return "model_obligation_divergence";
    }
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "invariant-failure"))
    {
        return "invariant_failure";
    }
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "temporal-failure"))
    {
        return "temporal_failure";
    }
    if run
        .events
        .iter()
        .any(|event| event.kind == "lost-message" || event.kind == "storage-crash-after-write")
    {
        return "lost-message";
    }
    if run
        .obligations
        .iter()
        .any(|obligation| unresolved_failure_mode(obligation) == "unresolved-reply")
    {
        return "unresolved-reply";
    }
    "unresolved-delivery"
}

pub(super) fn unresolved_failure_mode(
    obligation: &kobo_sim_core::RuntimeObligationSummary,
) -> &'static str {
    if obligation
        .actions
        .iter()
        .any(|action| action == "reply" || action == "reject" || action == "cancel")
    {
        "unresolved-reply"
    } else {
        "unresolved-delivery"
    }
}

pub(super) fn escaped_obligations(run: &FullDepthRun) -> Vec<String> {
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged && obligation.drop_span.is_some())
        .map(|obligation| obligation.binding.clone())
        .collect()
}

pub(super) fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    failure: &ScenarioFailure,
) -> Vec<serde_json::Value> {
    if failure.code != KErrorCode::K0100 {
        return Vec::new();
    }
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": span_json(source_path, source, obligation.declaration_span),
            })
        })
        .collect()
}

pub(super) fn span_json(
    source_path: &str,
    source: &str,
    span: (usize, usize),
) -> serde_json::Value {
    serde_json::json!({
        "path": source_path,
        "line": one_based_line_for_offset(source, span.0),
        "start": span.0,
        "end": span.1.max(span.0 + 1),
        "mapped": span.1 > span.0,
        "snippet": line_snippet(source, span.0),
    })
}

pub(super) fn boundary_decisions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "crate": decision.crate_name,
                "call_path": decision.call_path,
                "call_arguments": decision.call_arguments,
                "return_type": decision.return_type,
                "call_shape": decision.call_shape.as_str(),
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "source_span": {
                    "start": decision.span_start,
                    "end": decision.span_end,
                },
            })
        })
        .collect()
}

pub(super) fn events_json(events: &[ScenarioEvent]) -> Vec<serde_json::Value> {
    events
        .iter()
        .enumerate()
        .map(|(id, event)| {
            let mut value = serde_json::json!({
                "id": id,
                "kind": event.kind.clone(),
                "label": event.label.clone(),
                "value": event.value,
            });
            if let Some(io) = event.io.as_ref() {
                value
                    .as_object_mut()
                    .expect("event json should be an object")
                    .insert("io_capture".to_owned(), boundary_io_capture_json(io));
            }
            value
        })
        .collect()
}

pub(super) fn boundary_io_capture_json(
    capture: &kobo_sim_core::BoundaryIoCapture,
) -> serde_json::Value {
    serde_json::json!({
        "mode": capture.mode.clone(),
        "replay_key": capture.replay_key.clone(),
        "request": boundary_io_payload_json(&capture.request),
        "response": boundary_io_payload_json(&capture.response),
        "request_hash": capture.request_hash.clone(),
        "response_hash": capture.response_hash.clone(),
    })
}

pub(super) fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

pub(super) fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

pub(super) fn line_snippet(source: &str, offset: usize) -> String {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source[line_start..line_end].trim().to_owned()
}
