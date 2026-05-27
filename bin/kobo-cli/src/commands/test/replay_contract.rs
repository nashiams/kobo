use super::backend::backend_for_profile;
use super::*;
use crate::commands::ecosystem;

pub(super) fn validate_run_boundary_declarations(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> anyhow::Result<()> {
    for decision in &run.boundary_decisions {
        let policy = decision.policy.as_str();
        let adapter = config.ecosystem_policy.adapter_for(&decision.crate_name);
        if policy == "model" && adapter.is_none() {
            anyhow::bail!(
                "K0123: model boundary for `{}` has no adapter package",
                decision.crate_name
            );
        }
        if let Some(adapter) = adapter {
            if let Err(message) = ecosystem::validate_model_adapter_package(adapter) {
                anyhow::bail!(
                    "K0123: adapter package for `{}` failed validation: {message}",
                    decision.crate_name
                );
            }
        }
        if !matches!(policy, "typed" | "activity") {
            continue;
        }
        if let Err(error) = declarations::declaration_facts_for_boundary(
            file,
            config,
            &decision.crate_name,
            policy,
            decision.call_path.as_deref(),
        ) {
            let code = declaration_error_code(policy, error.key);
            anyhow::bail!(
                "{code}: configured {policy} metadata for `{}` failed validation at {} key `{}`: {}",
                decision.crate_name,
                error.path.display(),
                error.key,
                error.message
            );
        }
    }
    Ok(())
}

pub(super) fn declaration_error_code(policy: &str, key: &str) -> &'static str {
    match (policy, key) {
        ("activity", "activity") => "K0125",
        ("typed", "declaration") => "K0122",
        _ => "K0121",
    }
}

pub(super) fn replay_contract_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "scope": ecosystem_scope(run),
        "full_ecosystem_exploration": full_ecosystem_exploration(run),
        "facades": run
            .harness_manifest
            .as_ref()
            .map(|manifest| manifest.facades.clone())
            .unwrap_or_default(),
        "semantic_engine": run.digest.semantic_engine,
        "harness_engine": run.digest.harness_engine,
        "agreement": run.digest.agreement,
    })
}

pub(super) fn ecosystem_scope(run: &FullDepthRun) -> String {
    run.harness_manifest
        .as_ref()
        .map(|manifest| manifest.execution_scope.clone())
        .unwrap_or_else(|| "semantic-only".to_owned())
}

pub(super) fn full_ecosystem_exploration(run: &FullDepthRun) -> bool {
    run.harness_manifest
        .as_ref()
        .is_some_and(|manifest| manifest.full_ecosystem_exploration)
}

pub(super) fn exactness_json(run: &FullDepthRun) -> &'static str {
    match run.replay_guarantee {
        ReplayGuarantee::Exact => "exact",
        ReplayGuarantee::Partial => "partial",
        ReplayGuarantee::NotReplayable => "evidence_only",
    }
}

pub(super) struct ShrunkEventStream {
    pub(super) events: Vec<ScenarioEvent>,
    pub(super) removed_event_ids: Vec<usize>,
}

pub(super) fn shrink_event_stream(
    run: &FullDepthRun,
    _sim_profile: &str,
    shrink_mode: &str,
) -> ShrunkEventStream {
    let mut removed_event_ids = Vec::new();
    if shrink_mode == "best-effort" && run.replay_guarantee == ReplayGuarantee::Exact {
        removed_event_ids.extend(run.events.iter().enumerate().filter_map(|(index, event)| {
            if is_replay_irrelevant_event(&event.kind) {
                Some(index)
            } else {
                None
            }
        }));
    }
    let events = run
        .events
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed_event_ids.contains(index))
        .map(|(_, event)| event.clone())
        .collect();
    ShrunkEventStream {
        events,
        removed_event_ids,
    }
}

pub(super) fn shrink_json(
    run: &FullDepthRun,
    shrink_mode: &str,
    event_stream: &ShrunkEventStream,
) -> serde_json::Value {
    let mut shrink_passes = Vec::new();
    let scheduler_ids = removed_event_ids_for_kinds(run, event_stream, &["scheduler-pct-seed"]);
    if !scheduler_ids.is_empty() {
        shrink_passes.push(serde_json::json!({
            "pass": "trailing-independent-scheduler-events",
            "removed_event_ids": scheduler_ids,
            "replay_checked": true,
        }));
    }
    let network_ids =
        removed_event_ids_for_kinds(run, event_stream, &["network-delayed", "network-reordered"]);
    if !network_ids.is_empty() {
        shrink_passes.push(serde_json::json!({
            "pass": "independent-network-ordering",
            "removed_event_ids": network_ids,
            "replay_checked": true,
        }));
    }
    for (pass, reason) in [
        (
            "unused-events",
            "no replay-irrelevant semantic events found",
        ),
        ("seeds-runs", "no smaller equivalent seed/run pair found"),
        (
            "data-sizes",
            "scenario has no shrinkable generated data fixtures",
        ),
        (
            "independent-injections",
            "no independent injected hook can be removed without changing the trace",
        ),
    ] {
        shrink_passes.push(serde_json::json!({
            "pass": pass,
            "removed_event_ids": [],
            "replay_checked": run.replay_guarantee == ReplayGuarantee::Exact,
            "reason": reason,
        }));
    }
    serde_json::json!({
        "mode": shrink_mode,
        "original_event_count": run.events.len(),
        "shrunk_event_count": event_stream.events.len(),
        "removed_event_ids": event_stream.removed_event_ids.clone(),
        "shrink_passes": shrink_passes,
        "replay_checked": run.replay_guarantee == ReplayGuarantee::Exact,
        "reason_if_not_shrunk": if event_stream.removed_event_ids.is_empty() {
            Some("original witness is already minimal for the current replay invariant")
        } else {
            None
        },
    })
}

pub(super) fn removed_event_ids_for_kinds(
    run: &FullDepthRun,
    event_stream: &ShrunkEventStream,
    kinds: &[&str],
) -> Vec<usize> {
    event_stream
        .removed_event_ids
        .iter()
        .copied()
        .filter(|id| {
            run.events
                .get(*id)
                .is_some_and(|event| kinds.contains(&event.kind.as_str()))
        })
        .collect()
}

pub(super) fn is_replay_irrelevant_event(kind: &str) -> bool {
    matches!(
        kind,
        "scheduler-pct-seed" | "network-delayed" | "network-reordered" | "fuzz-shrink-candidate"
    )
}

pub(super) fn injections_json(inject: Option<&str>) -> serde_json::Value {
    let hooks = inject
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|hook| !hook.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    serde_json::json!({
        "hooks": hooks,
    })
}

pub(crate) fn backend_replay_id(
    mode: &str,
    source_identity: &str,
    seed: u64,
    run: &FullDepthRun,
) -> String {
    if mode == "none" {
        return "none".to_owned();
    }
    if let Some(harness_replay_id) = harness_backend_replay_id(run) {
        return harness_replay_id;
    }
    kobo_replay_hash(mode, source_identity, seed, run)
}

pub(crate) fn backend_replay_evidence_json(
    mode: &str,
    source_identity: &str,
    seed: u64,
    run: &FullDepthRun,
) -> serde_json::Value {
    let kobo_verification_hash = kobo_replay_hash(mode, source_identity, seed, run);
    let harness_replay_id = if mode == "none" {
        None
    } else {
        harness_backend_replay_id(run)
    };
    serde_json::json!({
        "backend": backend_for_profile(&run.profile),
        "mode": if harness_replay_id.is_some() { "generated_harness" } else { mode },
        "source": if harness_replay_id.is_some() { "generated-loom-harness" } else { "kobo-verification-hash" },
        "harness_replay_id": harness_replay_id,
        "kobo_verification_hash": kobo_verification_hash,
    })
}

pub(super) fn kobo_replay_hash(
    mode: &str,
    source_identity: &str,
    seed: u64,
    run: &FullDepthRun,
) -> String {
    match mode {
        "record" => replay_token(source_identity, seed, run),
        "metadata" => kobo_sim_core::digest::stable_hash(&format!(
            "metadata:{}:{}:{}:{}",
            source_identity,
            seed,
            backend_for_profile(&run.profile),
            run.digest.semantic_trace_hash
        )),
        "none" => "none".to_owned(),
        _ => replay_token(source_identity, seed, run),
    }
}

pub(super) fn harness_backend_replay_id(run: &FullDepthRun) -> Option<String> {
    if backend_for_profile(&run.profile) != "loom" {
        return None;
    }
    let manifest = run.harness_manifest.as_ref()?;
    let material = format!(
        "{}:{}:{}:{}:{}:{}",
        manifest.source_hash,
        manifest.generated_rust_hash,
        manifest.stdout_hash,
        manifest
            .checkpoint_artifact_hash
            .as_deref()
            .unwrap_or("no-checkpoint"),
        run.digest.semantic_trace_hash,
        run.digest.harness_trace_hash,
    );
    Some(format!(
        "loom-harness:{}",
        kobo_sim_core::digest::stable_hash(&material)
    ))
}

pub(super) fn replay_token(source_identity: &str, seed: u64, run: &FullDepthRun) -> String {
    let mut material = String::new();
    material.push_str(source_identity);
    material.push(':');
    material.push_str(&seed.to_string());
    material.push(':');
    material.push_str(backend_for_profile(&run.profile));
    material.push(':');
    material.push_str(&run.digest.semantic_trace_hash);
    material.push(':');
    material.push_str(&run.digest.harness_trace_hash);
    if let Some(fuzz_driver_trace_hash) = run.digest.fuzz_driver_trace_hash.as_deref() {
        material.push(':');
        material.push_str(fuzz_driver_trace_hash);
    }
    kobo_sim_core::digest::stable_hash(&material)
}
