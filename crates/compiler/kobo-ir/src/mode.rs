use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::GuaranteeProfile;

/// Legacy `script|checked|strict` input retained only for compatibility parsing.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegacyMode {
    #[default]
    Script,
    Checked,
    Strict,
}

impl LegacyMode {
    pub const fn guarantee_profile(self) -> GuaranteeProfile {
        match self {
            Self::Script => GuaranteeProfile::Dev,
            Self::Checked => GuaranteeProfile::Checked,
            Self::Strict => GuaranteeProfile::Release,
        }
    }
}

impl fmt::Display for LegacyMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Script => "script",
            Self::Checked => "checked",
            Self::Strict => "strict",
        })
    }
}

impl FromStr for LegacyMode {
    type Err = String;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        match source {
            "script" => Ok(Self::Script),
            "checked" => Ok(Self::Checked),
            "strict" => Ok(Self::Strict),
            _ => Err(format!(
                "invalid mode '{source}': valid values are 'script', 'checked', 'strict'",
            )),
        }
    }
}
