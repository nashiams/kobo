use kobo_ir::{FileEntry, FileSet, KoboSpan};

pub fn render_span_compact(file_set: &FileSet, span: KoboSpan) -> String {
    let Some(file) = file_set.get(span.file_id) else {
        return format!("<unknown>:{}..{}", span.start, span.end);
    };

    if !can_render_span_start(file, span) {
        return format!("{}:{}..{}", file.path.display(), span.start, span.end);
    }

    let (line, column) = file.line_col(span.start);
    format!("{}:{line}:{column}", file.path.display())
}

fn can_render_span_start(file: &FileEntry, span: KoboSpan) -> bool {
    span.start <= span.end
        && (span.start as usize) <= file.source().len()
        && file.line_text(file.line_col(span.start).0).is_some()
}
