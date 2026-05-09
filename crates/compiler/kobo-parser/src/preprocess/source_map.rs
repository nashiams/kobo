use kobo_ir::{FileId, KoboSpan};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PreprocessSourceMap {
    segments: Vec<PreprocessMapSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreprocessMapSegment {
    pub rewritten: KoboSpan,
    pub original: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreprocessedSource<T> {
    pub rewritten: String,
    pub source_map: PreprocessSourceMap,
    pub metadata: T,
}

impl PreprocessSourceMap {
    pub fn identity_for(source: &str, file_id: FileId) -> Self {
        let mut source_map = Self::default();
        source_map.push_segment(
            KoboSpan::new(0, source.len() as u32, file_id),
            KoboSpan::new(0, source.len() as u32, file_id),
        );
        source_map
    }

    pub fn push_segment(&mut self, rewritten: KoboSpan, original: KoboSpan) {
        if rewritten.start > rewritten.end || original.start > original.end {
            return;
        }
        self.segments.push(PreprocessMapSegment {
            rewritten,
            original,
        });
    }

    pub fn rewritten_span_to_original(&self, span: KoboSpan) -> Option<KoboSpan> {
        let segment = self
            .segments
            .iter()
            .filter(|segment| {
                segment.rewritten.file_id == span.file_id
                    && span.start >= segment.rewritten.start
                    && span.end <= segment.rewritten.end
            })
            .min_by_key(|segment| segment.rewritten.end - segment.rewritten.start)?;

        let rewritten_len = segment.rewritten.end - segment.rewritten.start;
        let original_len = segment.original.end - segment.original.start;
        let start_delta = span.start.saturating_sub(segment.rewritten.start);
        let end_delta = span.end.saturating_sub(segment.rewritten.start);

        let (original_start_delta, original_end_delta) = if rewritten_len == original_len {
            (start_delta, end_delta)
        } else if rewritten_len == 0 {
            (0, original_len)
        } else {
            (
                scale_offset(start_delta, rewritten_len, original_len),
                scale_offset(end_delta, rewritten_len, original_len),
            )
        };

        Some(KoboSpan::new(
            segment
                .original
                .start
                .saturating_add(original_start_delta)
                .min(segment.original.end),
            segment
                .original
                .start
                .saturating_add(original_end_delta)
                .min(segment.original.end),
            segment.original.file_id,
        ))
    }

    pub fn compose_with(&self, earlier: &PreprocessSourceMap) -> PreprocessSourceMap {
        let mut composed = PreprocessSourceMap::default();
        for segment in &self.segments {
            let original = earlier
                .rewritten_span_to_original(segment.original)
                .unwrap_or(segment.original);
            composed.push_segment(segment.rewritten, original);
        }
        composed
    }
}

pub(crate) fn push_identity_segment(
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    rewritten_start: usize,
    rewritten_end: usize,
    original_start: usize,
    original_end: usize,
) {
    if rewritten_start == rewritten_end && original_start == original_end {
        return;
    }
    source_map.push_segment(
        KoboSpan::new(rewritten_start as u32, rewritten_end as u32, file_id),
        KoboSpan::new(original_start as u32, original_end as u32, file_id),
    );
}

fn scale_offset(offset: u32, rewritten_len: u32, original_len: u32) -> u32 {
    let numerator = u64::from(offset) * u64::from(original_len);
    (numerator / u64::from(rewritten_len)).min(u64::from(original_len)) as u32
}
