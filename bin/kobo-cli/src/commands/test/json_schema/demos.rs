use kobo_errors::KErrorCode;
use kobo_ir::{ScenarioOpKind, ScenarioProgram};
use kobo_sim_core::{FullDepthRun, ReplayGuarantee};

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
pub(crate) struct FlagshipDemoFacts {
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

pub(crate) fn is_durable_queue_demo(facts: &FlagshipDemoFacts, run: &FullDepthRun) -> bool {
    has_lifecycle_actions(run, &["ack", "nack", "requeue"])
        && (facts.has_template("queue_delivery") || facts.has_storage_facade())
}

pub(crate) fn is_async_gateway_demo(facts: &FlagshipDemoFacts, run: &FullDepthRun) -> bool {
    has_lifecycle_actions(run, &["reply", "reject", "cancel"])
        && (run.profile == "async"
            || facts.has_template("handler_reply")
            || facts.has_modeled_boundary("ward.task")
            || has_event_kind(run, "scheduler-cancel-path")
            || has_event_kind(run, "failure-injection-cancel")
            || has_event_kind(run, "failure-injection-preempt"))
}

pub(crate) fn harness_built_cleanly(run: &FullDepthRun) -> bool {
    run.harness_manifest
        .as_ref()
        .is_some_and(|manifest| manifest.exit_code == 0 && !manifest.harness_rs_path.is_empty())
}

pub(crate) fn has_boundary_or_debt_evidence(run: &FullDepthRun) -> bool {
    !run.boundary_decisions.is_empty() || !run.opaque_boundaries.is_empty()
}

pub(crate) fn flagship_evidence_inputs(
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

pub(crate) fn has_event_kind(run: &FullDepthRun, kind: &str) -> bool {
    run.events.iter().any(|event| event.kind == kind)
        || run
            .failure
            .as_ref()
            .is_some_and(|failure| failure.events.iter().any(|event| event.kind == kind))
}

pub(crate) fn normalize_demo_action(action: &str) -> String {
    action.trim().replace('-', "_")
}

pub(crate) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|candidate| candidate == &value) {
        values.push(value);
    }
}

pub(crate) fn has_lifecycle_actions(run: &FullDepthRun, expected: &[&str]) -> bool {
    run.obligations.iter().any(|obligation| {
        expected
            .iter()
            .all(|action| obligation.actions.iter().any(|item| item == action))
    })
}

pub(crate) fn expanded_policy_json(profile: &str) -> serde_json::Value {
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

pub(crate) fn boundary_assumptions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
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
