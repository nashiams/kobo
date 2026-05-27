use super::*;

pub(crate) fn scheduler_json(
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

pub(crate) fn scheduler_seed_cases(run: &FullDepthRun, fallback_seed: u64) -> Vec<u64> {
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

pub(crate) fn scheduler_cancellation_json(run: &FullDepthRun) -> serde_json::Value {
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

pub(crate) fn scheduler_cancellation_events(run: &FullDepthRun) -> Vec<serde_json::Value> {
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

pub(crate) fn is_scheduler_cancellation_event(kind: &str) -> bool {
    matches!(
        kind,
        "scheduler-cancel-path" | "scheduler-future-dropped" | "failure-injection-cancel"
    )
}

pub(crate) fn modeled_boundaries_json(run: &FullDepthRun) -> Vec<&'static str> {
    run.modeled_boundaries
        .iter()
        .map(|boundary| boundary.as_str())
        .collect()
}
