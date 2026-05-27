use kobo_ir::KoboSpan;

use super::spans::spans_overlap;
use super::{KoboSourceMap, RsSpan};
impl KoboSourceMap {
    pub fn generated_file(&self) -> &str {
        &self.file
    }

    pub fn kobo_path(&self) -> &str {
        self.sources
            .first()
            .map(String::as_str)
            .unwrap_or(self.file.as_str())
    }

    pub fn lookup_kobo_span(&self, rs_span: RsSpan) -> Option<KoboSpan> {
        self.x_kobo_mappings
            .iter()
            .find(|entry| {
                entry.rs_span.line == rs_span.line && spans_overlap(&entry.rs_span, &rs_span)
            })
            .map(|entry| entry.kobo_span)
            .or_else(|| {
                self.x_kobo_mappings
                    .iter()
                    .find(|entry| entry.rs_span.line == rs_span.line)
                    .map(|entry| entry.kobo_span)
            })
    }

    pub fn lookup_rs_spans(&self, kobo_span: KoboSpan) -> Vec<RsSpan> {
        self.x_kobo_mappings
            .iter()
            .filter(|entry| entry.kobo_span.overlaps(kobo_span) || entry.kobo_span == kobo_span)
            .map(|entry| entry.rs_span.clone())
            .collect()
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
