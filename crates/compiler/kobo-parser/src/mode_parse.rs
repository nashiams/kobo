use kobo_ir::KoboMode;

/// Errors that can occur when parsing the file-level mode attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeParseError {
    /// Invalid mode string (not script/checked/strict)
    InvalidMode { line: usize, value: String },
    /// Duplicate mode attribute found
    DuplicateMode {
        first_line: usize,
        second_line: usize,
    },
}

/// Legacy `//! kobo:mode = ...` directive retained as a compatibility alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyModeDirective {
    pub line: usize,
    pub value: String,
    pub mode: KoboMode,
}

impl LegacyModeDirective {
    pub const fn profile_name(&self) -> &'static str {
        match self.mode {
            KoboMode::Script => "dev",
            KoboMode::Checked => "checked",
            KoboMode::Strict => "release",
        }
    }
}

impl std::fmt::Display for ModeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModeParseError::InvalidMode { line, value } => {
                write!(
                    f,
                    "line {}: invalid mode '{}': valid values are 'script', 'checked', 'strict'",
                    line, value
                )
            }
            ModeParseError::DuplicateMode {
                first_line,
                second_line,
            } => {
                write!(
                    f,
                    "duplicate mode attribute: first at line {}, second at line {}",
                    first_line, second_line
                )
            }
        }
    }
}

impl std::error::Error for ModeParseError {}

/// Scans the first 10 lines of source for `//! kobo:mode = script|checked|strict`.
///
/// Returns `Ok(None)` if no mode attribute is found.
/// Returns `Ok(Some(mode))` if exactly one valid attribute is found.
/// Returns `Err` on invalid mode string or duplicate attributes.
pub fn parse_file_mode(source: &str) -> Result<Option<KoboMode>, ModeParseError> {
    Ok(parse_legacy_mode_directive(source)?.map(|directive| directive.mode))
}

/// Scans the first 10 lines for a legacy `//! kobo:mode = ...` directive.
///
/// The directive is no longer the public source-level model. Callers that apply
/// it should also emit a migration diagnostic naming the equivalent profile.
pub fn parse_legacy_mode_directive(
    source: &str,
) -> Result<Option<LegacyModeDirective>, ModeParseError> {
    let mut found: Option<LegacyModeDirective> = None;

    for (idx, line) in source.lines().take(10).enumerate() {
        let line_num = idx + 1;
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("//!") {
            let rest = rest.trim();
            if let Some(mode_str) = rest.strip_prefix("kobo:mode") {
                let mode_str = mode_str.trim();
                if let Some(value) = mode_str.strip_prefix('=') {
                    let value = value.trim();
                    let mode: KoboMode =
                        value.parse().map_err(|_| ModeParseError::InvalidMode {
                            line: line_num,
                            value: value.to_owned(),
                        })?;

                    if let Some(first) = &found {
                        return Err(ModeParseError::DuplicateMode {
                            first_line: first.line,
                            second_line: line_num,
                        });
                    }
                    found = Some(LegacyModeDirective {
                        line: line_num,
                        value: value.to_owned(),
                        mode,
                    });
                }
            }
        }
    }

    Ok(found)
}
