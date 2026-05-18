use kobo_errors::KErrorCode;

use crate::core::{ScenarioEvent, ScenarioFailure};

#[derive(Clone, Debug, Default)]
pub(crate) struct NetworkModel {
    has_in_flight_message: bool,
    has_dropped_message: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct NetworkTransition {
    pub events: Vec<ScenarioEvent>,
    pub failure: Option<ScenarioFailure>,
}

impl NetworkModel {
    pub(crate) fn apply_action(
        &mut self,
        action: &str,
        seed: u64,
        span: (usize, usize),
    ) -> NetworkTransition {
        let normalized = normalize_action(action);
        let mut events = events_for_action(action, seed);
        match normalized.as_str() {
            "send" => {
                self.has_in_flight_message = true;
                self.has_dropped_message = false;
                events.push(state_event("network-queued", "in-flight", seed));
            }
            "delay" => {
                if self.has_in_flight_message && !self.has_dropped_message {
                    events.push(state_event("network-delayed", "in-flight", seed));
                }
            }
            "reorder" => {
                if self.has_in_flight_message && !self.has_dropped_message {
                    events.push(state_event("network-reordered", "in-flight", seed));
                }
            }
            "drop" => {
                if self.has_in_flight_message {
                    self.has_dropped_message = true;
                    events.push(state_event("network-dropped", "dropped", seed));
                }
            }
            "receive" => {
                if self.has_in_flight_message && !self.has_dropped_message {
                    self.has_in_flight_message = false;
                    events.push(state_event("network-delivered", "delivered", seed));
                } else {
                    return NetworkTransition {
                        events,
                        failure: Some(ScenarioFailure {
                            code: KErrorCode::K0100,
                            message:
                                "network delivery failed: receive observed dropped or missing message"
                                    .to_owned(),
                            primary_start: span.0,
                            primary_end: span.1,
                            events: vec![ScenarioEvent {
                                kind: "network-delivery-failed".to_owned(),
                                label: Some("ward.network".to_owned()),
                                value: None,
                                io: None,
                            }],
                        }),
                    };
                }
            }
            _ => {}
        }

        NetworkTransition {
            events,
            failure: None,
        }
    }
}

pub(crate) fn events_for_action(action: &str, seed: u64) -> Vec<ScenarioEvent> {
    vec![ScenarioEvent {
        kind: format!("network-{}", normalize_event_part(action)),
        label: Some("ward.network".to_owned()),
        value: Some(seed),
        io: None,
    }]
}

pub(crate) fn harness_events_for_action(action: &str, seed: u64) -> Vec<ScenarioEvent> {
    let mut events = events_for_action(action, seed);
    match normalize_action(action).as_str() {
        "send" => events.push(state_event("network-queued", "in-flight", seed)),
        "delay" => events.push(state_event("network-delayed", "in-flight", seed)),
        "reorder" => events.push(state_event("network-reordered", "in-flight", seed)),
        "drop" => events.push(state_event("network-dropped", "dropped", seed)),
        _ => {}
    }
    events
}

fn state_event(kind: &str, state: &str, seed: u64) -> ScenarioEvent {
    ScenarioEvent {
        kind: kind.to_owned(),
        label: Some(format!("ward.network:{state}")),
        value: Some(seed),
        io: None,
    }
}

fn normalize_action(action: &str) -> String {
    action
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn normalize_event_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
