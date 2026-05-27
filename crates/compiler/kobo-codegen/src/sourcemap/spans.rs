use super::RsSpan;

pub(super) fn rs_span_from_syn(span: proc_macro2::Span) -> RsSpan {
    let start = span.start();
    let end = span.end();
    RsSpan {
        line: start.line,
        column_start: start.column,
        column_end: end.column.max(start.column + 1),
    }
}

pub(super) fn spans_overlap(left: &RsSpan, right: &RsSpan) -> bool {
    left.column_start <= right.column_end && right.column_start <= left.column_end
}
