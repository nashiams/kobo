use kobo_sim_core::FullDepthRun;

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
    let configured_seed_count = configured_seed_count(run, seeds.len() as u64);
    let portfolio_cap = seed_portfolio_cap_json(run);
    serde_json::json!({
        "profile": sim_profile,
        "strategy": strategy,
        "seed": seed,
        "seed_count": seeds.len(),
        "configured_seed_count": configured_seed_count,
        "portfolio_complete": portfolio_cap.is_null() && seeds.len() as u64 >= configured_seed_count,
        "portfolio_cap": portfolio_cap,
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

fn configured_seed_count(run: &FullDepthRun, executed_seed_count: u64) -> u64 {
    run.events
        .iter()
        .filter(|event| {
            event.kind == "scheduler-seed-case" || event.kind == "scheduler-seed-portfolio-cap"
        })
        .filter_map(|event| event.label.as_deref().and_then(seed_count_from_label))
        .next()
        .unwrap_or(executed_seed_count)
}

fn seed_count_from_label(label: &str) -> Option<u64> {
    u64_from_label_field(label, "count")
}

fn u64_from_label_field(label: &str, field: &str) -> Option<u64> {
    let prefix = format!("{field}=");
    label
        .split(';')
        .find_map(|part| part.strip_prefix(&prefix))
        .and_then(|value| value.parse::<u64>().ok())
}

fn seed_portfolio_cap_json(run: &FullDepthRun) -> serde_json::Value {
    let Some(event) = run
        .events
        .iter()
        .find(|event| event.kind == "scheduler-seed-portfolio-cap")
    else {
        return serde_json::Value::Null;
    };
    let label = event.label.as_deref();
    serde_json::json!({
        "reason": "wall-clock",
        "elapsed_ms": event.value,
        "executed_seed_count": label.and_then(|value| u64_from_label_field(value, "executed")),
        "configured_seed_count": label.and_then(seed_count_from_label),
        "label": event.label,
    })
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
