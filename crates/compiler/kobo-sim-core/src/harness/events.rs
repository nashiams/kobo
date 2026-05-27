use crate::core::ScenarioEvent;
use crate::error::{Result, SimCoreError};
use crate::harness_manifest::ServiceHookEvent;

pub(super) fn event_marker() -> String {
    "KOBO_EVENT:".to_owned()
}

pub(super) fn event_print_statement(event: &ScenarioEvent) -> Result<String> {
    let json = serde_json::to_string(event)
        .map_err(|source| SimCoreError::json("serialize harness event", source))?;
    Ok(format!("println!(\"KOBO_EVENT:{{}}\", r#\"{json}\"#);"))
}

pub(super) fn event_print_statements(events: &[ScenarioEvent]) -> Result<String> {
    let statements = events
        .iter()
        .map(event_print_statement)
        .collect::<Result<Vec<_>>>()?;
    Ok(statements.join("\n        "))
}

pub(super) fn parse_harness_events(stdout: &str) -> Result<Vec<ScenarioEvent>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("KOBO_EVENT:"))
        .map(|json| {
            serde_json::from_str(json)
                .map_err(|source| SimCoreError::json("parse generated harness event", source))
        })
        .collect()
}

pub(super) fn parse_harness_service_hook_events(stdout: &str) -> Result<Vec<ServiceHookEvent>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("KOBO_SERVICE_HOOK:"))
        .map(|json| {
            serde_json::from_str(json)
                .map_err(|source| SimCoreError::json("parse generated service hook event", source))
        })
        .collect()
}
