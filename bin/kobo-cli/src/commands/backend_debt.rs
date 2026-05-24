use std::path::{Path, PathBuf};

use anyhow::Context;

use super::sim_model;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DebtControlSource {
    TestCommand,
    InspectHarness,
    Config,
}

impl DebtControlSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::TestCommand => "kobo test",
            Self::InspectHarness => "inspect --sim --harness",
            Self::Config => "Kobo.toml",
        }
    }
}

pub(super) fn backend_capability(
    backend: &str,
) -> Option<&'static kobo_sim_core::backend::BackendCapability> {
    kobo_sim_core::backend::capabilities()
        .iter()
        .find(|capability| capability.name == backend)
}

pub(super) fn unsupported_backend_capability(
    backend: &str,
) -> Option<&'static kobo_sim_core::backend::BackendCapability> {
    backend_capability(backend).filter(|capability| !capability.executes_now)
}

pub(super) struct ReservedBackendIntent<'a> {
    pub(super) capability: &'static kobo_sim_core::backend::BackendCapability,
    pub(super) scheduler: Option<&'a str>,
}

pub(super) fn configured_reserved_backend_intent<'a>(
    config: &'a kobo_driver::KoboConfig,
    profile: &str,
) -> Option<ReservedBackendIntent<'a>> {
    reserved_backend_fit_for_profile(profile)
        .iter()
        .filter_map(|backend| {
            let backend_config = config.sim.backends.get(*backend)?;
            if !backend_config.enabled || !configured_backend_has_controls(backend_config) {
                return None;
            }
            let capability = unsupported_backend_capability(backend)?;
            Some(ReservedBackendIntent {
                capability,
                scheduler: backend_config.scheduler.as_deref(),
            })
        })
        .next()
}

pub(super) fn write_unsupported_backend_debt(
    file: &Path,
    target_name: &str,
    capability: &kobo_sim_core::backend::BackendCapability,
    scheduler: Option<&str>,
    control_source: DebtControlSource,
) -> anyhow::Result<PathBuf> {
    let debt_dir = std::env::current_dir()
        .context("failed to determine current directory")?
        .join(".kobo")
        .join("scenario-debt");
    std::fs::create_dir_all(&debt_dir)
        .with_context(|| format!("failed to create {}", debt_dir.display()))?;
    let debt_path = debt_dir.join(format!(
        "{}-{}-backend.json",
        sanitize_name(target_name),
        sanitize_name(capability.name)
    ));
    let source_path = sim_model::cli_relative_path(file)?;
    let debt = serde_json::json!({
        "schema_version": 1,
        "status": "scenario-debt",
        "kind": "unsupported-backend-native-control",
        "target": format!("{source_path}:{target_name}"),
        "backend": capability.name,
        "display_name": capability.display_name,
        "scheduler": scheduler,
        "control_source": control_source.as_str(),
        "integration_level": capability.integration_level,
        "scenario_execution": capability.scenario_execution,
        "role": capability.role,
        "available_paths": [
            "use a stable Kobo profile",
            "keep the inspected backend-native harness",
            "keep this unsupported knob as scenario debt"
        ],
    });
    std::fs::write(&debt_path, serde_json::to_string_pretty(&debt)?)
        .with_context(|| format!("failed to write {}", debt_path.display()))?;
    Ok(debt_path)
}

pub(super) fn unsupported_backend_debt_message(
    capability: &kobo_sim_core::backend::BackendCapability,
    debt_path: &Path,
) -> anyhow::Result<String> {
    let debt_path = sim_model::cli_relative_path(debt_path)?;
    Ok(format!(
        "unsupported backend option `{}`: {} (adapter is not linked; {}, {}); use a stable Kobo profile, keep the inspected backend-native harness, or keep the unsupported knob as scenario debt recorded at {debt_path}",
        capability.name,
        capability.role,
        capability.integration_level,
        capability.scenario_execution
    ))
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn reserved_backend_fit_for_profile(profile: &str) -> &'static [&'static str] {
    match profile {
        "async" => &["shuttle"],
        "network" => &["turmoil"],
        "distributed" => &["madsim"],
        _ => &[],
    }
}

fn configured_backend_has_controls(config: &kobo_driver::SimBackendConfig) -> bool {
    config.scheduler.is_some()
        || config.replay_token.is_some()
        || config.max_branches.is_some()
        || config.checkpoint_replay.is_some()
}
