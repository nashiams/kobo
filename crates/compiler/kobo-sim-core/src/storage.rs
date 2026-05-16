use kobo_errors::KErrorCode;

use crate::core::{ScenarioEvent, ScenarioFailure};

#[derive(Clone, Debug, Default)]
pub(crate) struct StorageModel {
    has_delivery_ack: bool,
    has_pending_write: bool,
    has_durable_commit: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct StorageTransition {
    pub events: Vec<ScenarioEvent>,
    pub failure: Option<ScenarioFailure>,
}

impl StorageModel {
    pub(crate) fn note_obligation_action(&mut self, action: &str) {
        if matches!(action, "ack" | "reply") {
            self.has_delivery_ack = true;
        }
    }

    pub(crate) fn apply_action(
        &mut self,
        action: &str,
        seed: u64,
        span: (usize, usize),
    ) -> StorageTransition {
        let events = events_for_action(action, seed);
        match normalize_action(action).as_str() {
            "write" | "append" | "journal" => self.has_pending_write = true,
            "commit" | "flush" | "fsync" => self.has_durable_commit = true,
            "recover" | "replay" => {}
            _ => {}
        }

        let failure = if is_crash_action(action) {
            self.crash_failure(action, span)
        } else {
            None
        };
        StorageTransition { events, failure }
    }

    fn crash_failure(&self, action: &str, span: (usize, usize)) -> Option<ScenarioFailure> {
        if !(self.has_delivery_ack && self.has_pending_write && !self.has_durable_commit) {
            return None;
        }
        Some(ScenarioFailure {
            code: KErrorCode::K0100,
            message: "storage facade proved lost-message: ack before durable commit".to_owned(),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![ScenarioEvent {
                kind: "lost-message".to_owned(),
                label: Some(format!("storage-{}", normalize_event_part(action))),
                value: None,
            }],
        })
    }
}

pub(crate) fn events_for_action(action: &str, seed: u64) -> Vec<ScenarioEvent> {
    let event_kind = format!("storage-{}", normalize_event_part(action));
    let state_label = match normalize_action(action).as_str() {
        "write" | "append" | "journal" => "pending-write",
        "commit" | "flush" | "fsync" => "durable-commit",
        "recover" | "replay" => "recovery",
        action if action.contains("crash") => "crash",
        _ => "operation",
    };
    vec![ScenarioEvent {
        kind: event_kind,
        label: Some(format!("ward.storage:{state_label}")),
        value: Some(seed),
    }]
}

pub(crate) fn method_takes_value(action: &str) -> bool {
    !matches!(normalize_action(action).as_str(), "recover" | "replay")
        && !normalize_action(action).contains("crash")
}

fn is_crash_action(action: &str) -> bool {
    normalize_action(action).contains("crash")
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
