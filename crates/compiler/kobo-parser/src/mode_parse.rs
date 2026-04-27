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
    let mut found: Option<(usize, KoboMode)> = None;

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

                    if let Some((first_line, _)) = found {
                        return Err(ModeParseError::DuplicateMode {
                            first_line,
                            second_line: line_num,
                        });
                    }
                    found = Some((line_num, mode));
                }
            }
        }
    }

    Ok(found.map(|(_, mode)| mode))
}
