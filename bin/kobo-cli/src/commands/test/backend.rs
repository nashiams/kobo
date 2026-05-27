use super::{backend_debt, serde_json, EngineMode};
pub(super) struct ProfileRoles {
    pub(super) guarantee_profile: String,
    pub(super) backend_profile: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BackendExpertOptions {
    pub(super) backend: Option<String>,
    pub(super) scheduler: Option<String>,
    pub(super) max_branches: Option<u64>,
    pub(super) backend_native: bool,
}

impl BackendExpertOptions {
    pub(crate) const fn new(
        backend: Option<String>,
        scheduler: Option<String>,
        max_branches: Option<u64>,
        backend_native: bool,
    ) -> Self {
        Self {
            backend,
            scheduler,
            max_branches,
            backend_native,
        }
    }

    pub(super) fn backend_name<'a>(&'a self, backend_profile: &'a str) -> &'a str {
        self.backend
            .as_deref()
            .unwrap_or_else(|| backend_for_profile(backend_profile))
    }

    pub(super) fn has_explicit_backend_controls(&self) -> bool {
        self.backend.is_some()
            || self.scheduler.is_some()
            || self.max_branches.is_some()
            || self.backend_native
    }

    pub(super) fn execution_profile(&self, scenario_profile: &str) -> anyhow::Result<String> {
        let Some(backend) = self.backend.as_deref() else {
            return Ok(scenario_profile.to_owned());
        };
        let expected_profile = profile_for_backend(backend)?;
        if expected_profile != scenario_profile {
            anyhow::bail!(
                "unsupported backend option: backend `{backend}` requires simulation profile `{expected_profile}`, but the scenario resolved to `{scenario_profile}`; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
            );
        }
        Ok(expected_profile.to_owned())
    }

    pub(super) fn effective_scheduler<'a>(
        &'a self,
        config: &'a kobo_driver::KoboConfig,
        backend_profile: &'a str,
    ) -> Option<&'a str> {
        self.scheduler.as_deref().or_else(|| {
            config
                .sim
                .scheduler_for_backend(self.backend_name(backend_profile))
        })
    }

    pub(super) fn validate(
        &self,
        backend_profile: &str,
        sim_profile: &str,
        effective_scheduler: Option<&str>,
        effective_max_branches: Option<u64>,
    ) -> anyhow::Result<()> {
        let backend_name = self.backend_name(backend_profile);
        validate_backend_name(backend_name)?;
        if self.backend.is_some() {
            validate_backend_executes(backend_name)?;
        }
        validate_scheduler_control(backend_name, effective_scheduler, self.backend_native)?;
        validate_max_branches_control(backend_name, effective_max_branches)?;
        if self.backend_native && self.backend.is_none() {
            anyhow::bail!(
                "unsupported backend option: --backend-native requires --backend; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
            );
        }
        if self.backend_native && backend_name != "loom" {
            anyhow::bail!(
                "unsupported backend option: --backend-native is only supported by backend `loom`; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
            );
        }
        if effective_max_branches.is_some() && sim_profile != "exhaustive" {
            anyhow::bail!(
                "unsupported backend option: max_branches is only available with --sim exhaustive; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
            );
        }
        Ok(())
    }
}

pub(super) fn parse_engine(value: &str) -> anyhow::Result<EngineMode> {
    match value {
        "semantic" => Ok(EngineMode::SemanticOnly),
        "harness" => Ok(EngineMode::HarnessOnly),
        "both" => Ok(EngineMode::Both),
        _ => anyhow::bail!("invalid sim engine; expected semantic, harness, or both"),
    }
}

pub(super) fn validate_backend_name(backend: &str) -> anyhow::Result<()> {
    if backend_debt::backend_capability(backend).is_some() {
        Ok(())
    } else {
        anyhow::bail!(
            "unsupported backend option `{backend}`; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
        )
    }
}

pub(super) fn validate_backend_executes(backend: &str) -> anyhow::Result<()> {
    let Some(capability) = backend_debt::backend_capability(backend) else {
        validate_backend_name(backend)?;
        return Ok(());
    };
    if capability.executes_now {
        return Ok(());
    }
    anyhow::bail!(
        "unsupported backend option `{backend}`: {} (adapter is not linked; {}, {}); use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt",
        capability.role,
        capability.integration_level,
        capability.scenario_execution
    )
}

pub(super) fn profile_for_backend(backend: &str) -> anyhow::Result<&'static str> {
    match backend {
        "loom" => Ok("sync"),
        "shuttle" => Ok("async"),
        "turmoil" => Ok("network"),
        "madsim" => Ok("distributed"),
        "proptest" => Ok("stateful-input"),
        "failpoints" => Ok("failpoint"),
        _ => anyhow::bail!(
            "unsupported backend option `{backend}`; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
        ),
    }
}

pub(super) fn validate_scheduler_control(
    backend: &str,
    scheduler: Option<&str>,
    backend_native: bool,
) -> anyhow::Result<()> {
    let Some(scheduler) = scheduler else {
        return Ok(());
    };
    match (backend, scheduler) {
        ("loom", "exhaustive") => Ok(()),
        ("loom", "small-random") if !backend_native => Ok(()),
        ("loom", "small-random") => anyhow::bail!(
            "unsupported backend option: scheduler `small-random` is semantic-only for backend `loom`; use scheduler `exhaustive` for backend-native replay or mark unsupported knobs as scenario debt"
        ),
        ("shuttle" | "turmoil" | "madsim", _) => anyhow::bail!(
            "unsupported backend option: scheduler `{scheduler}` requires native backend `{backend}`, but that adapter is not linked; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
        ),
        _ => anyhow::bail!(
            "unsupported backend option: scheduler `{scheduler}` is not supported by backend `{backend}`; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
        ),
    }
}

pub(super) fn validate_max_branches_control(
    backend: &str,
    max_branches: Option<u64>,
) -> anyhow::Result<()> {
    if max_branches.is_none() || backend == "loom" {
        return Ok(());
    }
    anyhow::bail!(
        "unsupported backend option: --max-branches is only supported by backend `loom`; use a stable Kobo profile, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
    )
}

pub(super) fn backend_for_profile(profile: &str) -> &'static str {
    match profile {
        "sync" => "loom",
        "async" => "generated-rust-process",
        "stateful-input" => "proptest",
        "failpoint" => "failpoints",
        "network" => "network-loopback",
        "distributed" => "generated-rust-process",
        _ => "unknown",
    }
}

pub(super) fn reserved_backend_fit_for_profile(profile: &str) -> &'static [&'static str] {
    match profile {
        "async" => &["shuttle"],
        "network" => &["turmoil"],
        "distributed" => &["madsim"],
        _ => &[],
    }
}

pub(super) fn reserved_backend_fit_json(profile: &str) -> serde_json::Value {
    serde_json::Value::Array(
        reserved_backend_fit_for_profile(profile)
            .iter()
            .map(|backend| {
                let capability = kobo_sim_core::backend::capabilities()
                    .iter()
                    .find(|capability| capability.name == *backend);
                serde_json::json!({
                    "backend": backend,
                    "status": "reserved",
                    "integration_level": capability.map(|capability| capability.integration_level).unwrap_or("metadata-only"),
                    "scenario_execution": capability.map(|capability| capability.scenario_execution).unwrap_or("unsupported-native-adapter"),
                })
            })
            .collect(),
    )
}
