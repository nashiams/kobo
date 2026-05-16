use kobo_ir::{FileId, KoboSpan};

use super::source_map::{push_identity_segment, PreprocessSourceMap, PreprocessedSource};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcurrentSugarKind {
    Counter,
    Live,
    ViewDistance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcurrentSugarOccurrence {
    pub kind: ConcurrentSugarKind,
    pub original_span: (usize, usize),
    pub generated_span: (usize, usize),
}

struct SugarMatch {
    kind: ConcurrentSugarKind,
    marker: &'static str,
    line_start: usize,
    line_end: usize,
    keyword_start: usize,
    keyword_end: usize,
    indent: String,
    remainder: String,
}

pub fn preprocess_concurrent_sugar(source: &str) -> (String, Vec<ConcurrentSugarOccurrence>) {
    preprocess_concurrent_sugar_inner(source).map_or_else(
        || (source.to_owned(), Vec::new()),
        |(rewritten, occurrences, _)| (rewritten, occurrences),
    )
}

pub fn preprocess_concurrent_sugar_mapped(
    source: &str,
    file_id: FileId,
) -> PreprocessedSource<Vec<ConcurrentSugarOccurrence>> {
    let Some((rewritten, occurrences, replacements)) = preprocess_concurrent_sugar_inner(source)
    else {
        return PreprocessedSource {
            rewritten: source.to_owned(),
            source_map: PreprocessSourceMap::identity_for(source, file_id),
            metadata: Vec::new(),
        };
    };

    let mut source_map = PreprocessSourceMap::default();
    let mut original_cursor = 0usize;
    let mut rewritten_cursor = 0usize;

    for replacement in &replacements {
        push_identity_segment(
            &mut source_map,
            file_id,
            rewritten_cursor,
            replacement.generated_start,
            original_cursor,
            replacement.original_start,
        );
        source_map.push_segment(
            KoboSpan::new(
                replacement.generated_start as u32,
                replacement.generated_end as u32,
                file_id,
            ),
            KoboSpan::new(
                replacement.original_start as u32,
                replacement.original_end as u32,
                file_id,
            ),
        );
        original_cursor = replacement.original_end;
        rewritten_cursor = replacement.generated_end;
    }

    push_identity_segment(
        &mut source_map,
        file_id,
        rewritten_cursor,
        rewritten.len(),
        original_cursor,
        source.len(),
    );

    PreprocessedSource {
        rewritten,
        source_map,
        metadata: occurrences,
    }
}

#[derive(Clone, Debug)]
struct ReplacementRange {
    original_start: usize,
    original_end: usize,
    generated_start: usize,
    generated_end: usize,
}

fn preprocess_concurrent_sugar_inner(
    source: &str,
) -> Option<(
    String,
    Vec<ConcurrentSugarOccurrence>,
    Vec<ReplacementRange>,
)> {
    let mut rewritten = String::with_capacity(source.len() + 128);
    let mut occurrences = Vec::new();
    let mut replacements = Vec::new();
    let mut cursor = 0usize;

    for matched in iter_sugar_lines(source) {
        rewritten.push_str(&source[cursor..matched.line_start]);
        let generated_start = rewritten.len();
        let replacement = format!(
            "{}#[kobo::{}]\n{}let {}",
            matched.indent, matched.marker, matched.indent, matched.remainder
        );
        rewritten.push_str(&replacement);
        let generated_end = rewritten.len();
        occurrences.push(ConcurrentSugarOccurrence {
            kind: matched.kind,
            original_span: (matched.keyword_start, matched.keyword_end),
            generated_span: (generated_start, generated_end),
        });
        replacements.push(ReplacementRange {
            original_start: matched.line_start,
            original_end: matched.line_end,
            generated_start,
            generated_end,
        });
        cursor = matched.line_end;
    }

    if occurrences.is_empty() {
        return None;
    }

    rewritten.push_str(&source[cursor..]);
    Some((rewritten, occurrences, replacements))
}

fn iter_sugar_lines(source: &str) -> Vec<SugarMatch> {
    let mut matches = Vec::new();
    let mut line_start = 0usize;

    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if let Some(matched) = match_sugar_line(source, line, line_start, line_end) {
            matches.push(matched);
        }
        line_start = line_end;
    }

    if line_start < source.len() {
        let line = &source[line_start..];
        let line_end = source.len();
        if let Some(matched) = match_sugar_line(source, line, line_start, line_end) {
            matches.push(matched);
        }
    }

    matches
}

fn match_sugar_line(
    _source: &str,
    line: &str,
    line_start: usize,
    line_end: usize,
) -> Option<SugarMatch> {
    let trimmed_start = line.len() - line.trim_start_matches([' ', '\t']).len();
    let trimmed = &line[trimmed_start..];
    let (kind, keyword, marker) = sugar_keyword(trimmed)?;
    let after_keyword = &trimmed[keyword.len()..];
    if after_keyword
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_ascii_whitespace())
    {
        return None;
    }
    let remainder = after_keyword.trim_start_matches([' ', '\t']).to_owned();
    if !looks_like_let_binding(&remainder) {
        return None;
    }

    Some(SugarMatch {
        kind,
        marker,
        line_start,
        line_end,
        keyword_start: line_start + trimmed_start,
        keyword_end: line_start + trimmed_start + keyword.len(),
        indent: line[..trimmed_start].to_owned(),
        remainder,
    })
}

fn sugar_keyword(source: &str) -> Option<(ConcurrentSugarKind, &'static str, &'static str)> {
    [
        (ConcurrentSugarKind::Counter, "@counter", "counter"),
        (ConcurrentSugarKind::Live, "@live", "live"),
        (
            ConcurrentSugarKind::ViewDistance,
            "@view_distance",
            "view_distance",
        ),
    ]
    .into_iter()
    .find(|(_, keyword, _)| source.starts_with(keyword))
}

fn looks_like_let_binding(remainder: &str) -> bool {
    let without_comment = remainder.split("//").next().unwrap_or(remainder);
    without_comment.contains('=') && without_comment.trim_end().ends_with(';')
}

#[cfg(test)]
mod tests {
    use super::{preprocess_concurrent_sugar, ConcurrentSugarKind};

    #[test]
    fn rewrites_counter_live_and_view_distance_to_kobo_attrs() {
        let source = "fn f() {\n    @counter hits: u64 = 0;\n    @live route: String = String::new();\n    @view_distance radius: u32 = 64;\n}\n";

        let (rewritten, occurrences) = preprocess_concurrent_sugar(source);

        assert!(rewritten.contains("#[kobo::counter]\n    let hits: u64 = 0;"));
        assert!(rewritten.contains("#[kobo::live]\n    let route: String = String::new();"));
        assert!(rewritten.contains("#[kobo::view_distance]\n    let radius: u32 = 64;"));
        assert_eq!(occurrences.len(), 3);
        assert_eq!(occurrences[0].kind, ConcurrentSugarKind::Counter);
        assert_eq!(occurrences[1].kind, ConcurrentSugarKind::Live);
        assert_eq!(occurrences[2].kind, ConcurrentSugarKind::ViewDistance);
    }
}
