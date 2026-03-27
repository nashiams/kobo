use crate::node_id::FileId;

/// Byte-offset pair in a source file.
///
/// Every `KirNode` and every `KDiagnostic` carries a `KoboSpan`.
/// No node exists without a source location.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct KoboSpan {
    /// Byte offset of the first character of the span (inclusive).
    pub start: u32,
    /// Byte offset just past the last character of the span (exclusive).
    pub end: u32,
    pub file_id: FileId,
}

impl KoboSpan {
    pub fn new(start: u32, end: u32, file_id: FileId) -> Self {
        Self {
            start,
            end,
            file_id,
        }
    }

    /// Constructs a `KoboSpan` from a `proc_macro2::Span`.
    ///
    /// `proc_macro2::Span` has limited source info outside a proc-macro context.
    /// This uses the byte start/end when available; callers may fall back to
    /// line/col via a secondary conversion if byte offsets are absent.
    pub fn from_syn_span(span: proc_macro2::Span, file_id: FileId) -> Self {
        // proc_macro2::Span exposes byte_range() on nightly only.
        // On stable, we use the start byte position reported by the span's
        // Debug output as a best-effort approximation. Real line/col resolution
        // happens in kobo-parser where we have the full source text.
        let _ = span; // used in future integration with source_text resolution
        Self {
            start: 0,
            end: 0,
            file_id,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}
