/// Span conversion utility for @strict boundary scanners.
///
/// Converts `proc_macro2::Span` (line/column based) to `KoboSpan` (byte offset
/// based) using a pre-built `line_starts` table from the source text.
use kobo_ir::{FileId, KoboSpan};
use kobo_parser::KoboFile;

/// Lightweight converter from `proc_macro2::Span` → `KoboSpan`.
///
/// Created once per file, shared by all boundary scanners within the same
/// `validate_strict_boundary` call.
pub(crate) struct SpanConvert {
    line_starts: Vec<usize>,
    source_len: usize,
    pub file_id: FileId,
}

impl SpanConvert {
    /// Build from the full source text of a file (used in tests).
    #[allow(dead_code)]
    pub fn new(source: &str, file_id: FileId) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            line_starts,
            source_len: source.len(),
            file_id,
        }
    }

    /// Build from a `KoboFile`'s pre-computed line table.
    pub fn from_kobo_file(ast: &KoboFile) -> Self {
        Self {
            line_starts: ast.source_line_starts().to_vec(),
            source_len: ast.source_len(),
            file_id: ast.file_id,
        }
    }

    /// Convert a `proc_macro2::Span` to a `KoboSpan` with byte offsets.
    pub fn span(&self, syn_span: proc_macro2::Span) -> KoboSpan {
        KoboSpan::new(
            self.byte_offset(syn_span.start()) as u32,
            self.byte_offset(syn_span.end()) as u32,
            self.file_id,
        )
    }

    fn byte_offset(&self, lc: proc_macro2::LineColumn) -> usize {
        let line_idx = lc.line.saturating_sub(1);
        let line_start = self
            .line_starts
            .get(line_idx)
            .copied()
            .unwrap_or(self.source_len);
        (line_start + lc.column).min(self.source_len)
    }
}
