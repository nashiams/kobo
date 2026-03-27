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
    pub(crate) parsed_any: bool,
}

pub(crate) fn parse_rustc_errors(raw_output: &str) -> ParsedRustcOutput {
    let mut errors = Vec::new();
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
        }
    }

    ParsedRustcOutput { errors, parsed_any }
}
