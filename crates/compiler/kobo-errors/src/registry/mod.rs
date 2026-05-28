mod async_model;
mod ecosystem;
mod migration;
mod ownership;
mod parser;
mod performance;
mod replay;
mod solver;
mod strict_boundary;
use std::collections::BTreeMap;

use crate::{KErrorCode, Severity};
use kobo_ir::{GuaranteeLevel, GuaranteePolicy};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRegistry {
    entries: BTreeMap<&'static str, DiagnosticRegistryEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRegistryEntry {
    pub code: KErrorCode,
    pub code_text: &'static str,
    pub slug: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub explain: &'static str,
    pub category: DiagnosticCategory,
    pub status: DiagnosticStatus,
    pub default_severity: Severity,
    pub severity_policy: SeverityPolicy,
    pub mode_behavior: ModeBehavior,
    pub suggestion_policy: SuggestionPolicy,
    pub machine_edit_policy: MachineEditPolicy,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticCategory {
    Ownership,
    Performance,
    StrictBoundary,
    Async,
    Solver,
    Parser,
    RustcRemap,
    MigrationBoundary,
    Liveness,
    Nondeterminism,
    BoundaryPolicy,
    Replay,
    Simulation,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticStatus {
    Active,
    Reserved,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SeverityPolicy {
    Always(Severity),
    OwnershipModeDependent,
    AsyncModeDependent,
    ScriptDebtCheckedWarningStrictError,
    HiddenUntilActive,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ModeBehavior {
    Unspecified,
    NoModeDependency,
    OwnershipGuaranteeProfile,
    AsyncGuaranteeProfile,
    ScriptDebtStrictError,
    ReplayBoundaryPrompt,
    ParserRecovery,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SuggestionPolicy {
    Unspecified,
    None,
    HelpOnly,
    ReviewOnly,
    BoundaryPolicy,
    MachineApplicableAllowed,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MachineEditPolicy {
    Unspecified,
    NotApplicable,
    RefuseByDefault,
    AllowedWhenSuggestionMachineApplicable,
}

impl DiagnosticCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ownership => "ownership",
            Self::Performance => "performance",
            Self::StrictBoundary => "strict-boundary",
            Self::Async => "async",
            Self::Solver => "solver",
            Self::Parser => "parser",
            Self::RustcRemap => "rustc-remap",
            Self::MigrationBoundary => "migration-boundary",
            Self::Liveness => "liveness",
            Self::Nondeterminism => "nondeterminism",
            Self::BoundaryPolicy => "boundary-policy",
            Self::Replay => "replay",
            Self::Simulation => "simulation",
        }
    }
}

impl DiagnosticStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Reserved => "reserved",
        }
    }
}

impl SeverityPolicy {
    pub const fn resolve(self, policy: &GuaranteePolicy) -> Option<Severity> {
        match self {
            Self::Always(severity) => Some(severity),
            Self::OwnershipModeDependent => ownership_severity(policy),
            Self::AsyncModeDependent => async_severity(policy),
            Self::ScriptDebtCheckedWarningStrictError => debt_severity(policy),
            Self::HiddenUntilActive => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always(Severity::Error) => "always-error",
            Self::Always(Severity::Warning) => "always-warning",
            Self::Always(Severity::Note) => "always-note",
            Self::OwnershipModeDependent => "ownership-mode-dependent",
            Self::AsyncModeDependent => "async-mode-dependent",
            Self::ScriptDebtCheckedWarningStrictError => "script-debt-checked-warning-strict-error",
            Self::HiddenUntilActive => "hidden-until-active",
        }
    }
}

const fn ownership_severity(policy: &GuaranteePolicy) -> Option<Severity> {
    match policy.ownership_severity_level() {
        GuaranteeLevel::Off | GuaranteeLevel::Record => None,
        GuaranteeLevel::Checked => Some(Severity::Warning),
        GuaranteeLevel::Strict => Some(Severity::Error),
    }
}

const fn async_severity(policy: &GuaranteePolicy) -> Option<Severity> {
    match policy.async_severity_level() {
        GuaranteeLevel::Off | GuaranteeLevel::Record | GuaranteeLevel::Checked => {
            Some(Severity::Warning)
        }
        GuaranteeLevel::Strict => Some(Severity::Error),
    }
}

const fn debt_severity(policy: &GuaranteePolicy) -> Option<Severity> {
    match policy.debt_severity_level() {
        GuaranteeLevel::Off | GuaranteeLevel::Record | GuaranteeLevel::Checked => {
            Some(Severity::Warning)
        }
        GuaranteeLevel::Strict => Some(Severity::Error),
    }
}

impl ModeBehavior {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::NoModeDependency => "no mode dependency",
            Self::OwnershipGuaranteeProfile => "ownership guarantee profile",
            Self::AsyncGuaranteeProfile => "async guarantee profile",
            Self::ScriptDebtStrictError => "Script debt, Strict error",
            Self::ReplayBoundaryPrompt => "normal Rust allowed; replay requires boundary policy",
            Self::ParserRecovery => "parser recovery",
        }
    }
}

impl SuggestionPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::None => "none",
            Self::HelpOnly => "help-only",
            Self::ReviewOnly => "review-only",
            Self::BoundaryPolicy => "boundary-policy",
            Self::MachineApplicableAllowed => "machine-applicable-allowed",
        }
    }
}

impl MachineEditPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::NotApplicable => "not-applicable",
            Self::RefuseByDefault => "refuse-by-default",
            Self::AllowedWhenSuggestionMachineApplicable => {
                "allowed-when-suggestion-machine-applicable"
            }
        }
    }
}

impl DiagnosticRegistry {
    pub fn active_entries(&self) -> impl Iterator<Item = &DiagnosticRegistryEntry> {
        self.entries
            .values()
            .filter(|entry| entry.status == DiagnosticStatus::Active)
    }

    pub fn get(&self, code: KErrorCode) -> Option<&DiagnosticRegistryEntry> {
        self.find_by_code_text(code.as_str())
    }

    pub fn find_by_code_text(&self, code_text: &str) -> Option<&DiagnosticRegistryEntry> {
        let normalized = normalize_code_text(code_text);
        self.entries.get(normalized.as_str())
    }

    pub fn nearest_code_text(&self, code_text: &str) -> Option<&'static str> {
        let target = code_number(&normalize_code_text(code_text))?;
        self.entries
            .keys()
            .filter_map(|code| code_number(code).map(|number| (*code, number.abs_diff(target))))
            .min_by_key(|(_, distance)| *distance)
            .map(|(code, _)| code)
    }
}

pub fn diagnostic_registry() -> DiagnosticRegistry {
    let mut entries = BTreeMap::new();
    for entry in registry_entries() {
        entries.insert(entry.code_text, entry);
    }
    DiagnosticRegistry { entries }
}

fn registry_entries() -> Vec<DiagnosticRegistryEntry> {
    let mut entries = Vec::new();
    entries.extend(ownership::entries());
    entries.extend(performance::entries());
    entries.extend(strict_boundary::entries());
    entries.extend(async_model::entries());
    entries.extend(solver::entries());
    entries.extend(migration::entries());
    entries.extend(replay::entries());
    entries.extend(parser::entries());
    entries.extend(ecosystem::entries());
    append_reserved_entries(&mut entries);
    entries
}

fn append_reserved_entries(entries: &mut Vec<DiagnosticRegistryEntry>) {
    for code in KErrorCode::ALL {
        if !entries.iter().any(|entry| entry.code == *code) {
            entries.push(reserved_entry(*code));
        }
    }
}

fn reserved_entry(code: KErrorCode) -> DiagnosticRegistryEntry {
    DiagnosticRegistryEntry {
        code,
        code_text: code.as_str(),
        slug: "reserved-kobo-diagnostic-slot",
        title: "reserved Kobo diagnostic slot",
        summary: "This K-code is reserved by this Kobo version and is not emitted as a user diagnostic yet.",
        explain: "Kobo keeps stable K-code ranges so future diagnostics can be added without renumbering existing errors. A reserved slot is part of that public catalog, but normal compiler output should not produce it until the slot is promoted into an active diagnostic.",
        category: reserved_category(code),
        status: DiagnosticStatus::Reserved,
        default_severity: Severity::Note,
        severity_policy: SeverityPolicy::HiddenUntilActive,
        mode_behavior: ModeBehavior::NoModeDependency,
        suggestion_policy: SuggestionPolicy::HelpOnly,
        machine_edit_policy: MachineEditPolicy::NotApplicable,
    }
}

fn reserved_category(code: KErrorCode) -> DiagnosticCategory {
    match code_number(code.as_str()).unwrap_or_default() {
        1..=19 => DiagnosticCategory::Ownership,
        20..=39 => DiagnosticCategory::Performance,
        40..=59 => DiagnosticCategory::StrictBoundary,
        60..=79 => DiagnosticCategory::Async,
        80..=89 => DiagnosticCategory::Solver,
        90..=99 => DiagnosticCategory::MigrationBoundary,
        100..=109 => DiagnosticCategory::Replay,
        110..=113 => DiagnosticCategory::Parser,
        114..=118 => DiagnosticCategory::Replay,
        _ => DiagnosticCategory::Simulation,
    }
}

fn entry(
    code: KErrorCode,
    slug: &'static str,
    title: &'static str,
    summary: &'static str,
    explain: &'static str,
    category: DiagnosticCategory,
    default_severity: Severity,
    severity_policy: SeverityPolicy,
    mode_behavior: ModeBehavior,
    suggestion_policy: SuggestionPolicy,
    machine_edit_policy: MachineEditPolicy,
) -> DiagnosticRegistryEntry {
    DiagnosticRegistryEntry {
        code,
        code_text: code.as_str(),
        slug,
        title,
        summary,
        explain,
        category,
        status: DiagnosticStatus::Active,
        default_severity,
        severity_policy,
        mode_behavior,
        suggestion_policy,
        machine_edit_policy,
    }
}

fn normalize_code_text(code_text: &str) -> String {
    code_text.trim().to_ascii_uppercase()
}

fn code_number(code_text: &str) -> Option<u32> {
    let digits = code_text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}
