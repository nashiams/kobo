use std::path::Path;

use kobo_ir::ScenarioProgram;
use kobo_sim_core::FullDepthRun;

use super::super::{declarations, formal_core, summary_validation};
use super::events::boundary_io_capture_json;
use super::scheduler::modeled_boundaries_json;

pub(crate) fn boundary_policies_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
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

pub(crate) fn ecosystem_boundaries_json(
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

pub(crate) fn declaration_metadata_for_boundary(
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

pub(crate) fn activity_metadata_for_boundary(
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

pub(crate) fn declarations_json(
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

pub(crate) fn declaration_metadata_json(
    facts: &declarations::DeclarationFacts,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("path".to_owned(), facts.path.display().to_string().into());
    object.insert("version".to_owned(), facts.version.clone().into());
    object.insert(
        "schema_version".to_owned(),
        serde_json::json!(facts.schema_version),
    );
    object.insert("hash".to_owned(), facts.hash.clone().into());
    if let Some(package) = facts.metadata_package.as_ref() {
        object.insert(
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
    serde_json::Value::Object(object)
}

pub(crate) fn summary_usage_json(
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

pub(crate) fn boundary_evidence_for_policy(
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

pub(crate) fn boundary_capture_json(
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

pub(crate) fn boundary_decision_io_capture_json(
    decision: &kobo_sim_core::BoundaryDecision,
) -> Option<serde_json::Value> {
    if !matches!(decision.policy.as_str(), "record" | "activity") {
        return None;
    }
    let capture = decision.recorded_io.as_ref()?;
    Some(boundary_io_capture_json(capture))
}

pub(crate) fn boundary_io_payload_json(
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

pub(crate) fn boundary_event_label(decision: &kobo_sim_core::BoundaryDecision) -> String {
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
