use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_ir::{
    ErrorPolicy, GuaranteeDimension, GuaranteeDowngrade, GuaranteeLevel, GuaranteePolicy,
    GuaranteeProfile,
};
use toml::Value as TomlValue;

use crate::{ErrorFormat, GuaranteeProfileArg};

#[derive(Clone, Debug)]
struct ReleasePolicy {
    is_configured: bool,
    deny_new_debt: bool,
    strict_paths: Vec<String>,
    deny_downgrade_without_reason: bool,
}

#[derive(Clone, Debug)]
pub(super) struct EffectiveGuaranteePolicy {
    policy: GuaranteePolicy,
    release: ReleasePolicy,
    downgrade: Option<GuaranteeDowngrade>,
}

impl ReleasePolicy {
    const fn default() -> Self {
        Self {
            is_configured: false,
            deny_new_debt: true,
            strict_paths: Vec::new(),
            deny_downgrade_without_reason: true,
        }
    }

    fn apply_table(&mut self, table: &toml::map::Map<String, TomlValue>) {
        self.is_configured = true;
        if let Some(value) = table.get("deny_new_debt").and_then(TomlValue::as_bool) {
            self.deny_new_debt = value;
        }
        if let Some(value) = table
            .get("deny_downgrade_without_reason")
            .and_then(TomlValue::as_bool)
        {
            self.deny_downgrade_without_reason = value;
        }
        if let Some(paths) = table.get("strict_paths").and_then(TomlValue::as_array) {
            self.strict_paths = paths
                .iter()
                .filter_map(TomlValue::as_str)
                .map(str::to_owned)
                .collect();
        }
    }
}

impl EffectiveGuaranteePolicy {
    pub(super) fn compiler_policy(&self) -> &GuaranteePolicy {
        &self.policy
    }

    pub(super) fn downgrade(&self) -> Option<&GuaranteeDowngrade> {
        self.downgrade.as_ref()
    }

    pub(super) fn error_policy_name(&self) -> &'static str {
        self.policy.guarantees().errors().as_str()
    }

    pub(super) const fn has_configured_release_policy(&self) -> bool {
        self.release.is_configured
    }

    pub(super) const fn denies_new_debt(&self) -> bool {
        self.release.is_configured && self.release.deny_new_debt
    }
}

pub(super) fn load_effective_policy(
    file: Option<&Path>,
    profile: GuaranteeProfileArg,
) -> anyhow::Result<EffectiveGuaranteePolicy> {
    let root = policy_root(file)?;
    let manifest = read_manifest(&root)?;
    let mut policy = GuaranteePolicy::for_profile(profile.compiler_profile());
    let mut release = ReleasePolicy::default();

    if let Some(table) = table_at(&manifest, &["guarantees"]) {
        apply_guarantees_table(&mut policy, table);
    }
    if let Some(table) = table_at(&manifest, &["profiles", profile.as_str(), "guarantees"]) {
        apply_guarantees_table(&mut policy, table);
    }
    if let Some(table) = table_at(&manifest, &["ci", "release"]) {
        release.apply_table(table);
    }

    let mut downgrade = None;
    if let Some(file) = file {
        let relative = relative_policy_path(file, &root);
        if let Some(paths) = table_at(&manifest, &["paths"]) {
            for (pattern, value) in paths {
                if !matches_policy_pattern(&relative, pattern) {
                    continue;
                }
                let Some(table) = value.as_table() else {
                    continue;
                };
                let before = policy.guarantees().clone();
                apply_guarantees_table(&mut policy, table);
                let has_reason = table
                    .get("reason")
                    .and_then(TomlValue::as_str)
                    .is_some_and(|reason| !reason.trim().is_empty());
                if release.deny_downgrade_without_reason && !has_reason {
                    downgrade = policy.guarantees().first_downgrade_from(&before);
                }
            }
        }
        if profile.compiler_profile() == GuaranteeProfile::Release
            && release
                .strict_paths
                .iter()
                .any(|pattern| matches_policy_pattern(&relative, pattern))
        {
            raise_policy_to_release_floor(&mut policy);
        }
    }

    Ok(EffectiveGuaranteePolicy {
        policy,
        release,
        downgrade,
    })
}

pub(super) fn load_configured_release_policy(
    file: Option<&Path>,
    base_policy: &GuaranteePolicy,
) -> anyhow::Result<Option<EffectiveGuaranteePolicy>> {
    let profile = profile_arg_for(base_policy.profile());
    let loaded = load_effective_policy(file, profile)?;
    if loaded.has_configured_release_policy() {
        Ok(Some(loaded))
    } else {
        Ok(None)
    }
}

const fn profile_arg_for(profile: GuaranteeProfile) -> GuaranteeProfileArg {
    match profile {
        GuaranteeProfile::Dev => GuaranteeProfileArg::Dev,
        GuaranteeProfile::Checked => GuaranteeProfileArg::Checked,
        GuaranteeProfile::Release => GuaranteeProfileArg::Release,
    }
}

fn raise_policy_to_release_floor(policy: &mut GuaranteePolicy) {
    let release = GuaranteePolicy::for_profile(GuaranteeProfile::Release);
    let release_guarantees = release.guarantees();
    if policy.guarantees().ownership() < release_guarantees.ownership() {
        policy.guarantees_mut().set_level(
            GuaranteeDimension::Ownership,
            release_guarantees.ownership(),
        );
    }
    if policy.guarantees().liveness() < release_guarantees.liveness() {
        policy
            .guarantees_mut()
            .set_level(GuaranteeDimension::Liveness, release_guarantees.liveness());
    }
    if policy.guarantees().replay() < release_guarantees.replay() {
        policy
            .guarantees_mut()
            .set_level(GuaranteeDimension::Replay, release_guarantees.replay());
    }
    if policy.guarantees().boundaries() < release_guarantees.boundaries() {
        policy.guarantees_mut().set_level(
            GuaranteeDimension::Boundaries,
            release_guarantees.boundaries(),
        );
    }
    if policy.guarantees().errors() < release_guarantees.errors() {
        policy
            .guarantees_mut()
            .set_errors(release_guarantees.errors());
    }
}

pub(super) fn print_policy_json(policy: &EffectiveGuaranteePolicy) -> anyhow::Result<()> {
    let guarantees = policy.policy.guarantees();
    let value = serde_json::json!({
        "profile": policy.policy.profile().as_str(),
        "guarantees": {
            "ownership": guarantees.ownership().as_str(),
            "liveness": guarantees.liveness().as_str(),
            "replay": guarantees.replay().as_str(),
            "boundaries": guarantees.boundaries().as_str(),
            "errors": guarantees.errors().as_str(),
        },
        "ci": {
            "release": {
                "deny_new_debt": policy.release.deny_new_debt,
                "strict_paths": policy.release.strict_paths,
                "deny_downgrade_without_reason": policy.release.deny_downgrade_without_reason,
            }
        }
    });
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

pub(super) fn emit_policy_summary(policy: &EffectiveGuaranteePolicy) {
    let guarantees = policy.policy.guarantees();
    eprintln!(
        "guarantee profile `{}`: ownership={}, liveness={}, replay={}, boundaries={}, errors={}",
        policy.policy.profile().as_str(),
        guarantees.ownership().as_str(),
        guarantees.liveness().as_str(),
        guarantees.replay().as_str(),
        guarantees.boundaries().as_str(),
        guarantees.errors().as_str()
    );
}

pub(super) fn emit_downgrade(
    downgrade: &GuaranteeDowngrade,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    match error_format {
        ErrorFormat::Json => {
            let value = serde_json::json!({
                "kind": "guarantee_policy_downgrade",
                "dimension": downgrade.dimension,
                "from": downgrade.from,
                "to": downgrade.to,
                "reason_required": downgrade.reason_required,
                "message": format!(
                    "guarantee policy downgrade for {} from {} to {} requires a reason ledger entry",
                    downgrade.dimension, downgrade.from, downgrade.to
                ),
            });
            println!("{}", serde_json::to_string(&value)?);
        }
        ErrorFormat::Human => {
            eprintln!(
                "guarantee policy downgrade for {} from {} to {} requires a reason ledger entry",
                downgrade.dimension, downgrade.from, downgrade.to
            );
        }
    }
    Ok(())
}

fn apply_guarantees_table(policy: &mut GuaranteePolicy, table: &toml::map::Map<String, TomlValue>) {
    if let Some(level) = table
        .get("ownership")
        .and_then(TomlValue::as_str)
        .and_then(GuaranteeLevel::parse)
    {
        policy
            .guarantees_mut()
            .set_level(GuaranteeDimension::Ownership, level);
    }
    if let Some(level) = table
        .get("liveness")
        .and_then(TomlValue::as_str)
        .and_then(GuaranteeLevel::parse)
    {
        policy
            .guarantees_mut()
            .set_level(GuaranteeDimension::Liveness, level);
    }
    if let Some(level) = table
        .get("replay")
        .and_then(TomlValue::as_str)
        .and_then(GuaranteeLevel::parse)
    {
        policy
            .guarantees_mut()
            .set_level(GuaranteeDimension::Replay, level);
    }
    if let Some(level) = table
        .get("boundaries")
        .and_then(TomlValue::as_str)
        .and_then(GuaranteeLevel::parse)
    {
        policy
            .guarantees_mut()
            .set_level(GuaranteeDimension::Boundaries, level);
    }
    if let Some(errors) = table
        .get("errors")
        .and_then(TomlValue::as_str)
        .and_then(ErrorPolicy::parse)
    {
        policy.guarantees_mut().set_errors(errors);
    }
}

fn read_manifest(root: &Path) -> anyhow::Result<TomlValue> {
    let path = root.join("Kobo.toml");
    if !path.is_file() {
        return Ok(TomlValue::Table(toml::map::Map::new()));
    }
    let source = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    source
        .parse::<TomlValue>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn table_at<'a>(
    value: &'a TomlValue,
    path: &[&str],
) -> Option<&'a toml::map::Map<String, TomlValue>> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_table()
}

fn policy_root(file: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(file) = file {
        let mut start = absolute_path(file)?;
        if start.is_file() {
            start.pop();
        }
        for ancestor in start.ancestors() {
            if ancestor.join("Kobo.toml").is_file() {
                return Ok(ancestor.to_path_buf());
            }
        }
    }

    std::env::current_dir().context("failed to determine current directory")
}

fn absolute_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(path))
    }
}

fn relative_policy_path(file: &Path, root: &Path) -> String {
    let absolute = absolute_path(file).unwrap_or_else(|_| file.to_path_buf());
    let relative = absolute.strip_prefix(root).unwrap_or(&absolute);
    normalize_path(relative)
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn matches_policy_pattern(relative: &str, pattern: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return relative == prefix || relative.starts_with(&format!("{prefix}/"));
    }
    relative == pattern
}

impl From<GuaranteeProfileArg> for GuaranteeProfile {
    fn from(profile: GuaranteeProfileArg) -> Self {
        profile.compiler_profile()
    }
}
