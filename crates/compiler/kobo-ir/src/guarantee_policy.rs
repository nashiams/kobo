use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuaranteeProfile {
    #[default]
    Dev,
    Checked,
    Release,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuaranteeLevel {
    Off,
    Record,
    Checked,
    Strict,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorPolicy {
    Ergonomic,
    Typed,
    Explicit,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum GuaranteeDimension {
    Ownership,
    Liveness,
    Replay,
    Boundaries,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct GuaranteeSet {
    ownership: GuaranteeLevel,
    liveness: GuaranteeLevel,
    replay: GuaranteeLevel,
    boundaries: GuaranteeLevel,
    errors: ErrorPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct GuaranteePolicy {
    profile: GuaranteeProfile,
    guarantees: GuaranteeSet,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct GuaranteeDowngrade {
    pub dimension: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub reason_required: bool,
}

impl GuaranteeProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Checked => "checked",
            Self::Release => "release",
        }
    }
}

impl fmt::Display for GuaranteeProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GuaranteeProfile {
    type Err = String;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        match source {
            "dev" => Ok(Self::Dev),
            "checked" => Ok(Self::Checked),
            "release" => Ok(Self::Release),
            _ => Err(format!(
                "invalid profile '{source}': valid values are 'dev', 'checked', 'release'"
            )),
        }
    }
}

impl GuaranteeLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Record => "record",
            Self::Checked => "checked",
            Self::Strict => "strict",
        }
    }

    pub fn parse(source: &str) -> Option<Self> {
        match source {
            "off" => Some(Self::Off),
            "record" => Some(Self::Record),
            "checked" => Some(Self::Checked),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }
}

impl ErrorPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ergonomic => "ergonomic",
            Self::Typed => "typed",
            Self::Explicit => "explicit",
        }
    }

    pub fn parse(source: &str) -> Option<Self> {
        match source {
            "ergonomic" => Some(Self::Ergonomic),
            "typed" => Some(Self::Typed),
            "explicit" => Some(Self::Explicit),
            _ => None,
        }
    }
}

impl GuaranteeSet {
    pub const fn for_profile(profile: GuaranteeProfile) -> Self {
        match profile {
            GuaranteeProfile::Dev => Self {
                ownership: GuaranteeLevel::Record,
                liveness: GuaranteeLevel::Record,
                replay: GuaranteeLevel::Record,
                boundaries: GuaranteeLevel::Record,
                errors: ErrorPolicy::Ergonomic,
            },
            GuaranteeProfile::Checked => Self {
                ownership: GuaranteeLevel::Checked,
                liveness: GuaranteeLevel::Checked,
                replay: GuaranteeLevel::Checked,
                boundaries: GuaranteeLevel::Checked,
                errors: ErrorPolicy::Typed,
            },
            GuaranteeProfile::Release => Self {
                ownership: GuaranteeLevel::Strict,
                liveness: GuaranteeLevel::Checked,
                replay: GuaranteeLevel::Checked,
                boundaries: GuaranteeLevel::Strict,
                errors: ErrorPolicy::Explicit,
            },
        }
    }

    pub const fn ownership(&self) -> GuaranteeLevel {
        self.ownership
    }

    pub const fn liveness(&self) -> GuaranteeLevel {
        self.liveness
    }

    pub const fn replay(&self) -> GuaranteeLevel {
        self.replay
    }

    pub const fn boundaries(&self) -> GuaranteeLevel {
        self.boundaries
    }

    pub const fn errors(&self) -> ErrorPolicy {
        self.errors
    }

    pub fn set_level(&mut self, dimension: GuaranteeDimension, level: GuaranteeLevel) {
        match dimension {
            GuaranteeDimension::Ownership => self.ownership = level,
            GuaranteeDimension::Liveness => self.liveness = level,
            GuaranteeDimension::Replay => self.replay = level,
            GuaranteeDimension::Boundaries => self.boundaries = level,
        }
    }

    pub fn set_errors(&mut self, errors: ErrorPolicy) {
        self.errors = errors;
    }

    pub fn first_downgrade_from(&self, previous: &Self) -> Option<GuaranteeDowngrade> {
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

impl GuaranteePolicy {
    pub fn for_profile(profile: GuaranteeProfile) -> Self {
        Self {
            profile,
            guarantees: GuaranteeSet::for_profile(profile),
        }
    }

    pub const fn profile(&self) -> GuaranteeProfile {
        self.profile
    }

    pub const fn guarantees(&self) -> &GuaranteeSet {
        &self.guarantees
    }

    pub fn guarantees_mut(&mut self) -> &mut GuaranteeSet {
        &mut self.guarantees
    }

    pub const fn is_dev(&self) -> bool {
        matches!(self.profile, GuaranteeProfile::Dev)
    }

    pub const fn is_checked(&self) -> bool {
        matches!(self.profile, GuaranteeProfile::Checked)
    }

    pub const fn is_release(&self) -> bool {
        matches!(self.profile, GuaranteeProfile::Release)
    }

    pub const fn diag_always_active(&self) -> bool {
        matches!(self.profile, GuaranteeProfile::Checked)
    }

    pub const fn ownership_severity_level(&self) -> GuaranteeLevel {
        self.guarantees.ownership
    }

    pub const fn async_severity_level(&self) -> GuaranteeLevel {
        self.guarantees.ownership
    }

    pub const fn debt_severity_level(&self) -> GuaranteeLevel {
        self.guarantees.ownership
    }
}

impl Default for GuaranteePolicy {
    fn default() -> Self {
        Self::for_profile(GuaranteeProfile::Dev)
    }
}

impl GuaranteeDowngrade {
    pub const fn new(dimension: &'static str, from: &'static str, to: &'static str) -> Self {
        Self {
            dimension,
            from,
            to,
            reason_required: true,
        }
    }
}
