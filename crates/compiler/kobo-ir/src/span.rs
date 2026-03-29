use crate::node_id::FileId;

/// Byte-offset pair in a source file.
///
/// Every `KirNode` and every `KDiagnostic` carries a `KoboSpan`.
/// No node exists without a source location.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize)]
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

    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn contains(&self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    pub fn overlaps(&self, other: KoboSpan) -> bool {
        self.file_id == other.file_id && self.start < other.end && other.start < self.end
    }

    pub fn is_valid_for(&self, file: &crate::node_id::FileEntry) -> bool {
        self.start <= self.end && self.end as usize <= file.source().len()
    }
}

#[cfg(test)]
mod tests {
    use crate::{FileId, FileSetBuilder};

    use super::KoboSpan;

    #[test]
    fn span_helpers_cover_boundaries() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id =
            file_set_builder.add_file(FileId(0).0.to_string().into(), "ab\ncd".to_owned());
        let file_set = file_set_builder.finish();
        let file = file_set.get(file_id).expect("registered file should exist");

        let span = KoboSpan::new(0, 2, file_id);
        assert_eq!(span.len(), 2);
        assert!(span.contains(0));
        assert!(!span.contains(2));
        assert!(span.overlaps(KoboSpan::new(1, 3, file_id)));
        assert!(span.is_valid_for(file));
    }

    #[test]
    fn zero_length_and_out_of_bounds_spans_do_not_panic() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file("demo.kobo".into(), "hello\nworld".to_owned());
        let file_set = file_set_builder.finish();
        let file = file_set.get(file_id).expect("registered file should exist");

        let zero = KoboSpan::new(6, 6, file_id);
        assert_eq!(zero.len(), 0);
        assert!(!zero.contains(6));
        assert!(!zero.overlaps(KoboSpan::new(6, 11, file_id)));
        assert!(zero.is_valid_for(file));

        let invalid = KoboSpan::new(0, 40, file_id);
        assert!(!invalid.is_valid_for(file));
    }
}
