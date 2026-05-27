use kobo_codegen::RsSpan;
use kobo_ir::KoboSpan;
use kobo_parser::PreprocessSourceMap;

use crate::protocol::ProtocolRange;

pub(crate) fn original_span_for(source_map: &PreprocessSourceMap, span: KoboSpan) -> KoboSpan {
    source_map.rewritten_span_to_original(span).unwrap_or(span)
}

pub(crate) fn line_range_from_span(source: &str, span: KoboSpan) -> (usize, usize, usize) {
    let range = protocol_range_from_span(source, span);
    (range.line, range.character_start, range.character_end)
}

pub(crate) fn protocol_range_from_span(source: &str, span: KoboSpan) -> ProtocolRange {
    let start = span.start as usize;
    let end = span.end.max(span.start + 1) as usize;
    let mut line = 0usize;
    let mut line_start = 0usize;
    for (index, byte) in source.as_bytes().iter().enumerate() {
        if index >= start {
            break;
        }
        if *byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    let character_start = start.saturating_sub(line_start);
    let character_end = end.saturating_sub(line_start).max(character_start + 1);
    ProtocolRange {
        line,
        character_start,
        character_end,
    }
}

pub(crate) fn protocol_range_from_rs_span(rs_span: RsSpan) -> ProtocolRange {
    ProtocolRange {
        line: rs_span.line,
        character_start: rs_span.column_start,
        character_end: rs_span.column_end.max(rs_span.column_start + 1),
    }
}
