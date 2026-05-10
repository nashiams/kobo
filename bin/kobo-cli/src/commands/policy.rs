use std::path::{Path, PathBuf};

use anyhow::Context;
use toml::Value as TomlValue;

use crate::{ErrorFormat, GuaranteeProfileArg};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum GuaranteeLevel {
    Off,
    Record,
    Checked,
    Strict,
}

impl GuaranteeLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Record => "record",
            Self::Checked => "checked",
            Self::Strict => "strict",
        }
    }

    fn parse(value: &TomlValue) -> Option<Self> {
        match value.as_str()? {
            "off" => Some(Self::Off),
            "record" => Some(Self::Record),
            "checked" => Some(Self::Checked),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum ErrorPolicy {
    Ergonomic,
    Typed,
    Explicit,
}

impl ErrorPolicy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ergonomic => "ergonomic",
            Self::Typed => "typed",
            Self::Explicit => "explicit",
        }
    }

    fn parse(value: &TomlValue) -> Option<Self> {
        match value.as_str()? {
            "ergonomic" => Some(Self::Ergonomic),
            "typed" => Some(Self::Typed),
            "explicit" => Some(Self::Explicit),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct GuaranteeSet {
    ownership: GuaranteeLevel,
    liveness: GuaranteeLevel,
    replay: GuaranteeLevel,
    boundaries: GuaranteeLevel,
    errors: ErrorPolicy,
}

impl GuaranteeSet {
    const fn for_profile(profile: GuaranteeProfileArg) -> Self {
        match profile {
            GuaranteeProfileArg::Dev => Self {
                ownership: GuaranteeLevel::Record,
                liveness: GuaranteeLevel::Record,
                replay: GuaranteeLevel::Record,
                boundaries: GuaranteeLevel::Record,
                errors: ErrorPolicy::Ergonomic,
            },
            GuaranteeProfileArg::Checked => Self {
                ownership: GuaranteeLevel::Checked,
                liveness: GuaranteeLevel::Checked,
                replay: GuaranteeLevel::Checked,
                boundaries: GuaranteeLevel::Checked,
                errors: ErrorPolicy::Typed,
            },
            GuaranteeProfileArg::Release => Self {
                ownership: GuaranteeLevel::Strict,
                liveness: GuaranteeLevel::Checked,
                replay: GuaranteeLevel::Checked,
                boundaries: GuaranteeLevel::Strict,
                errors: ErrorPolicy::Explicit,
            },
        }
    }

    fn apply_table(&mut self, table: &toml::map::Map<String, TomlValue>) {
        if let Some(value) = table.get("ownership").and_then(GuaranteeLevel::parse) {
            self.ownership = value;
        }
        if let Some(value) = table.get("liveness").and_then(GuaranteeLevel::parse) {
            self.liveness = value;
        }
        if let Some(value) = table.get("replay").and_then(GuaranteeLevel::parse) {
            self.replay = value;
        }
        if let Some(value) = table.get("boundaries").and_then(GuaranteeLevel::parse) {
            self.boundaries = value;
        }
        if let Some(value) = table.get("errors").and_then(ErrorPolicy::parse) {
            self.errors = value;
        }
    }

    fn first_downgrade_from(&self, previous: &Self) -> Option<GuaranteeDowngrade> {
        if self.ownership < previous.ownership {
            return Some(GuaranteeDowngrade::new(
                "ownership",
                previous.ownership.as_str(),
                self.ownership.as_str(),
            ));
        }
        if self.liveness < previous.liveness {
            return Some(GuaranteeDowngrade::new(
                "liveness",
                previous.liveness.as_str(),
                self.liveness.as_str(),
            ));
        }
        if self.replay < previous.replay {
            return Some(GuaranteeDowngrade::new(
                "replay",
                previous.replay.as_str(),
                self.replay.as_str(),
            ));
        }
        if self.boundaries < previous.boundaries {
            return Some(GuaranteeDowngrade::new(
                "boundaries",
                previous.boundaries.as_str(),
                self.boundaries.as_str(),
            ));
        }
        if self.errors < previous.errors {
            return Some(GuaranteeDowngrade::new(
                "errors",
                previous.errors.as_str(),
                self.errors.as_str(),
            ));
        }
        None
    }
}

#[derive(Clone, Debug)]
struct ReleasePolicy {
    deny_new_debt: bool,
    strict_paths: Vec<String>,
    deny_downgrade_without_reason: bool,
}

impl ReleasePolicy {
    const fn default() -> Self {
        Self {
            deny_new_debt: true,
            strict_paths: Vec::new(),
            deny_downgrade_without_reason: true,
        }
    }

    fn apply_table(&mut self, table: &toml::map::Map<String, TomlValue>) {
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

#[derive(Clone, Debug)]
pub(super) struct GuaranteePolicy {
    profile: GuaranteeProfileArg,
    guarantees: GuaranteeSet,
    release: ReleasePolicy,
    downgrade: Option<GuaranteeDowngrade>,
}

impl GuaranteePolicy {
    pub(super) fn downgrade(&self) -> Option<&GuaranteeDowngrade> {
        self.downgrade.as_ref()
    }

    pub(super) fn error_policy_name(&self) -> &'static str {
        self.guarantees.errors.as_str()
    }
}

#[derive(Clone, Debug)]
pub(super) struct GuaranteeDowngrade {
    dimension: &'static str,
    from: &'static str,
    to: &'static str,
    reason_required: bool,
}

impl GuaranteeDowngrade {
    const fn new(dimension: &'static str, from: &'static str, to: &'static str) -> Self {
        Self {
            dimension,
            from,
            to,
            reason_required: true,
        }
    }
}

pub(super) fn load_effective_policy(
    file: Option<&Path>,
    profile: GuaranteeProfileArg,
) -> anyhow::Result<GuaranteePolicy> {
    let root = policy_root(file)?;
    let manifest = read_manifest(&root)?;
    let mut guarantees = GuaranteeSet::for_profile(profile);
    let mut release = ReleasePolicy::default();

    if let Some(table) = table_at(&manifest, &["guarantees"]) {
        guarantees.apply_table(table);
    }
    if let Some(table) = table_at(&manifest, &["profiles", profile.as_str(), "guarantees"]) {
        guarantees.apply_table(table);
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
                let before = guarantees.clone();
                guarantees.apply_table(table);
                let has_reason = table
                    .get("reason")
                    .and_then(TomlValue::as_str)
                    .is_some_and(|reason| !reason.trim().is_empty());
                if release.deny_downgrade_without_reason && !has_reason {
                    downgrade = guarantees.first_downgrade_from(&before);
                }
            }
        }
    }

    Ok(GuaranteePolicy {
        profile,
        guarantees,
        release,
        downgrade,
    })
}

pub(super) fn print_policy_json(policy: &GuaranteePolicy) -> anyhow::Result<()> {
    let value = serde_json::json!({
        "profile": policy.profile.as_str(),
        "guarantees": {
            "ownership": policy.guarantees.ownership.as_str(),
            "liveness": policy.guarantees.liveness.as_str(),
            "replay": policy.guarantees.replay.as_str(),
            "boundaries": policy.guarantees.boundaries.as_str(),
            "errors": policy.guarantees.errors.as_str(),
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

pub(super) fn emit_policy_summary(policy: &GuaranteePolicy) {
    eprintln!(
        "guarantee profile `{}`: ownership={}, liveness={}, replay={}, boundaries={}, errors={}",
        policy.profile.as_str(),
        policy.guarantees.ownership.as_str(),
        policy.guarantees.liveness.as_str(),
        policy.guarantees.replay.as_str(),
        policy.guarantees.boundaries.as_str(),
        policy.guarantees.errors.as_str()
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
