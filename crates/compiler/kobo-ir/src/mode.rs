use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

/// Compilation mode for the current Kobo session.
///
/// Shared vocabulary used by the pipeline, formatter, and driver.
/// Single source of truth — do NOT create a parallel enum [Contract R01].
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KoboMode {
    #[default]
    Script,
    Checked,
    Strict,
}

impl KoboMode {
    pub fn is_script(&self) -> bool {
        *self == KoboMode::Script
    }

    pub fn is_checked(&self) -> bool {
        *self == KoboMode::Checked
    }

    pub fn is_strict(&self) -> bool {
        *self == KoboMode::Strict
    }

    /// Whether DiagOwner instrumentation is unconditionally active in this mode.
    /// Mode check is authoritative; KOBO_DIAG=0 does NOT override checked mode [Contract R03].
    pub fn diag_always_active(&self) -> bool {
        matches!(self, KoboMode::Checked)
    }
}

impl fmt::Display for KoboMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Script => "script",
            Self::Checked => "checked",
            Self::Strict => "strict",
        })
    }
}

impl FromStr for KoboMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "script" => Ok(Self::Script),
            "checked" => Ok(Self::Checked),
            "strict" => Ok(Self::Strict),
            _ => Err(format!(
                "invalid mode '{}': valid values are 'script', 'checked', 'strict'",
                s
            )),
        }
    }
}
