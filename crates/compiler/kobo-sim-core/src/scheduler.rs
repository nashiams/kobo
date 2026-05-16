use kobo_errors::KErrorCode;

use crate::core::{ModeledBoundary, ScenarioEvent, ScenarioFailure, ScenarioOptions};

pub(crate) fn modeled_boundary_events(
    boundary: &ModeledBoundary,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    let mut events = vec![deterministic_boundary_event(boundary, options.seed)];
    if matches!(boundary, ModeledBoundary::WardTask) {
        events.push(ScenarioEvent {
            kind: "task-schedule-point".to_owned(),
            label: Some(options.sim_profile.clone()),
            value: Some(options.seed),
        });
        if options.sim_profile == "deep" {
            events.push(ScenarioEvent {
                kind: "scheduler-choice".to_owned(),
                label: Some("deep:pct-preempt".to_owned()),
                value: Some(options.seed.rotate_left(7)),
            });
        }
    }
    events
}

pub(crate) fn schedule_failure(
    boundary: &ModeledBoundary,
    options: &ScenarioOptions,
    active_obligation: Option<(&str, (usize, usize))>,
    span: (usize, usize),
) -> Option<ScenarioFailure> {
    if !matches!(options.sim_profile.as_str(), "deep" | "exhaustive")
        || !matches!(boundary, ModeledBoundary::WardTask)
    {
        return None;
    }
    let (binding, obligation_span) = active_obligation?;
    Some(ScenarioFailure {
        code: KErrorCode::K0100,
        message: format!(
            "scheduler deep interleaving exposed obligation `{binding}` across task boundary"
        ),
        primary_start: span.0,
        primary_end: span.1,
        events: vec![ScenarioEvent {
            kind: "scheduler-interleaving-leak".to_owned(),
            label: Some(binding.to_owned()),
            value: Some(obligation_span.0 as u64),
        }],
    })
}

pub(crate) fn portfolio_events(options: &ScenarioOptions) -> Vec<ScenarioEvent> {
    let budget = scheduler_budget(options);
    let mut events = vec![ScenarioEvent {
        kind: "scheduler-portfolio".to_owned(),
        label: Some(options.sim_profile.clone()),
        value: Some(budget),
    }];
    if options.sim_profile == "deep" {
        events.push(ScenarioEvent {
            kind: "scheduler-pct-seed".to_owned(),
            label: Some("deep".to_owned()),
            value: Some(options.seed.rotate_left(7)),
        });
    }
    if options.sim_profile == "exhaustive" {
        events.push(ScenarioEvent {
            kind: "scheduler-exhaustive-cap".to_owned(),
            label: Some("tiny-ward".to_owned()),
            value: Some(budget),
        });
    }
    events
}

pub(crate) fn scheduler_budget(options: &ScenarioOptions) -> u64 {
    options
        .event_budget
        .unwrap_or(match options.sim_profile.as_str() {
            "quick" => 64,
            "deep" => 1024,
            "replay" => 64,
            "exhaustive" => 16,
            _ => 64,
        })
}

fn deterministic_boundary_event(boundary: &ModeledBoundary, seed: u64) -> ScenarioEvent {
    match boundary {
        ModeledBoundary::WardTime => ScenarioEvent {
            kind: "deterministic-time".to_owned(),
            label: None,
            value: Some(seed.wrapping_mul(1_000).wrapping_add(17)),
        },
        ModeledBoundary::WardRandom => ScenarioEvent {
            kind: "deterministic-random".to_owned(),
            label: None,
            value: Some(seed.rotate_left(13) ^ 0x9e37_79b9_7f4a_7c15_u64),
        },
        ModeledBoundary::WardTask => ScenarioEvent {
            kind: "deterministic-task".to_owned(),
            label: Some("ward.task".to_owned()),
            value: Some(seed),
        },
        ModeledBoundary::WardStorage => ScenarioEvent {
            kind: "storage-boundary".to_owned(),
            label: Some("ward.storage".to_owned()),
            value: Some(seed),
        },
        ModeledBoundary::WardNetwork => ScenarioEvent {
            kind: "network-boundary".to_owned(),
            label: Some("ward.network".to_owned()),
            value: Some(seed),
        },
    }
}
