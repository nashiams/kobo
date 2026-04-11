use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RustcCode {
    pub(crate) code: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RustcSpan {
    #[allow(dead_code)]
    pub(crate) file_name: String,
    pub(crate) line_start: usize,
    pub(crate) column_start: usize,
    #[allow(dead_code)]
    pub(crate) line_end: usize,
    pub(crate) column_end: usize,
    pub(crate) is_primary: bool,
    pub(crate) label: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RustcJsonError {
    pub(crate) message: String,
    pub(crate) code: Option<RustcCode>,
    pub(crate) level: String,
    pub(crate) spans: Vec<RustcSpan>,
    pub(crate) children: Vec<RustcJsonError>,
}

pub(crate) struct ParsedRustcOutput {
    pub(crate) errors: Vec<RustcJsonError>,
    pub(crate) warnings: Vec<RustcJsonError>,   // v0.6 G4: captured for checked-mode filtering
    pub(crate) parsed_any: bool,
}

pub(crate) fn parse_rustc_diagnostics(raw_output: &str) -> ParsedRustcOutput {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut parsed_any = false;

    for line in raw_output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let Ok(message) = serde_json::from_str::<RustcJsonError>(line) else {
            continue;
        };
        parsed_any = true;

        if message.level == "error" {
            errors.push(message);
        } else if message.level == "warning" {
            warnings.push(message);
        }
    }

    ParsedRustcOutput { errors, warnings, parsed_any }
}

#[cfg(test)]
mod tests {
    use super::parse_rustc_diagnostics;

    fn error_line(msg: &str, code: &str) -> String {
        format!(
            r#"{{"message":"{msg}","code":{{"code":"{code}"}},"level":"error","spans":[],"children":[]}}"#
        )
    }

    fn warning_line(msg: &str, lint: &str) -> String {
        format!(
            r#"{{"message":"{msg}","code":{{"code":"{lint}"}},"level":"warning","spans":[],"children":[]}}"#
        )
    }

    #[test]
    fn test_parse_rustc_diagnostics_mixed_error_and_warnings() {
        // 1 error + 2 warnings
        let raw = format!(
            "{}\n{}\n{}",
            error_line("use of moved value", "E0382"),
            warning_line("unused import: rc::Rc", "unused_imports"),
            warning_line("dead code: __kobo_guard_0", "dead_code"),
        );
        let out = parse_rustc_diagnostics(&raw);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.warnings.len(), 2);
        assert!(out.parsed_any);
    }

    #[test]
    fn test_parse_rustc_diagnostics_warnings_only() {
        // 0 errors + 3 warnings
        let raw = format!(
            "{}\n{}\n{}",
            warning_line("w1", "unused_imports"),
            warning_line("w2", "dead_code"),
            warning_line("w3", "clippy::redundant_clone"),
        );
        let out = parse_rustc_diagnostics(&raw);
        assert!(out.errors.is_empty());
        assert_eq!(out.warnings.len(), 3);
        assert!(out.parsed_any);
    }

    #[test]
    fn test_parse_rustc_diagnostics_empty_input() {
        let out = parse_rustc_diagnostics("");
        assert!(out.errors.is_empty());
        assert!(out.warnings.is_empty());
        assert!(!out.parsed_any);
    }
}
