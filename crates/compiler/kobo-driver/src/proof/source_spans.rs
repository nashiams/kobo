use super::{KoboSpan, SourceSpan};

pub(super) fn source_span_from_kobo(source_path: &str, source: &str, span: KoboSpan) -> SourceSpan {
    source_span_from_range(source_path, source, span.start as usize, span.end as usize)
}

pub(super) fn source_span_for_binding(
    source_path: &str,
    source: &str,
    binding: &str,
) -> SourceSpan {
    let let_binding = format!("let {binding}");
    let let_mut_binding = format!("let mut {binding}");
    let start = source
        .find(&let_binding)
        .or_else(|| source.find(&let_mut_binding))
        .unwrap_or(0);
    source_span_from_range(
        source_path,
        source,
        start,
        start.saturating_add(binding.len()),
    )
}

pub(super) fn source_span_from_range(
    source_path: &str,
    source: &str,
    start: usize,
    end: usize,
) -> SourceSpan {
    let bounded_start = start.min(source.len());
    let bounded_end = end.max(bounded_start + 1).min(source.len());
    SourceSpan {
        path: source_path.to_owned(),
        line: one_based_line_for_offset(source, bounded_start),
        start: bounded_start,
        end: bounded_end,
        mapped: bounded_end > bounded_start,
        snippet: line_snippet(source, bounded_start),
    }
}

pub(super) fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

pub(super) fn line_snippet(source: &str, offset: usize) -> String {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source[line_start..line_end].trim().to_owned()
}
