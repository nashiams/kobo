use kobo_ir::{FileId, KoboSpan};
use kobo_parser::KoboFile;
use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;
use syn::{Expr, ExprCall, ExprMethodCall, ExprTry, Path as SynPath};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ErrorPolicySite {
    pub source_span: KoboSpan,
    pub generated_offset: usize,
    pub line: usize,
    pub operation: String,
    pub variant: String,
    pub source_error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ErrorPolicyMarker {
    pub id: usize,
    pub source_span: KoboSpan,
    pub operation: String,
    pub variant: String,
    pub source_error: String,
}

impl ErrorPolicyMarker {
    pub(crate) fn from_try_expr(id: usize, kobo_file: &KoboFile, try_expr: &ExprTry) -> Self {
        let source_span =
            normalized_span(kobo_file.span_from_syn(try_expr.span()), kobo_file.file_id);
        let (operation, variant, source_error) = classify_try_operation(try_expr.expr.as_ref());
        Self {
            id,
            source_span,
            operation: operation.to_owned(),
            variant: variant.to_owned(),
            source_error: source_error.to_owned(),
        }
    }

    pub(crate) fn binding_name(&self) -> String {
        marker_binding_name(self.id)
    }

    pub(crate) fn method_name(&self) -> String {
        marker_method_name(self.id)
    }
}

pub(crate) fn marker_binding_name(id: usize) -> String {
    format!("__kobo_error_policy_site_{id}")
}

pub(crate) fn marker_method_name(id: usize) -> String {
    format!("__kobo_error_policy_site_{id}_map")
}

pub(crate) fn resolve_marked_error_policy_sites(
    marked_source: String,
    markers: &[ErrorPolicyMarker],
) -> (String, Vec<ErrorPolicySite>) {
    let mut source = marked_source;
    let mut sites = Vec::with_capacity(markers.len());
    for marker in markers {
        remove_marker_binding(&mut source, marker);
        if let Some(generated_offset) = remove_marker_method(&mut source, marker) {
            sites.push(ErrorPolicySite {
                source_span: marker.source_span,
                generated_offset,
                line: one_based_line_for_offset(&source, generated_offset),
                operation: marker.operation.clone(),
                variant: marker.variant.clone(),
                source_error: marker.source_error.clone(),
            });
        }
    }
    (source, sites)
}

fn remove_marker_binding(source: &mut String, marker: &ErrorPolicyMarker) {
    let binding = format!("let {} = ();", marker.binding_name());
    let Some(binding_start) = source.find(&binding) else {
        return;
    };
    let line_start = source[..binding_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[binding_start..]
        .find('\n')
        .map_or(source.len(), |index| binding_start + index + 1);
    source.replace_range(line_start..line_end, "");
}

fn remove_marker_method(source: &mut String, marker: &ErrorPolicyMarker) -> Option<usize> {
    let method = format!(".{}()", marker.method_name());
    let generated_offset = source.find(&method)?;
    source.replace_range(generated_offset..generated_offset + method.len(), "");
    Some(generated_offset)
}

fn normalized_span(span: KoboSpan, file_id: FileId) -> KoboSpan {
    if span.is_empty() {
        KoboSpan::new(span.start, span.start.saturating_add(1), file_id)
    } else {
        span
    }
}

fn classify_try_operation(expr: &Expr) -> (&'static str, &'static str, &'static str) {
    if expr_is_call_named(expr, "read_to_string") || expr_is_method_named(expr, "read_to_string") {
        ("read_to_string", "ReadToString", "std::io::Error")
    } else if expr_is_call_named(expr, "write") || expr_is_method_named(expr, "write_all") {
        ("write", "Write", "std::io::Error")
    } else if expr_is_call_named(expr, "open") {
        ("open", "Open", "std::io::Error")
    } else {
        ("io", "Io", "std::io::Error")
    }
}

fn expr_is_call_named(expr: &Expr, expected: &str) -> bool {
    let Expr::Call(ExprCall { func, .. }) = expr else {
        return false;
    };
    let Expr::Path(path) = func.as_ref() else {
        return false;
    };
    path_last_ident(&path.path).as_deref() == Some(expected)
}

fn expr_is_method_named(expr: &Expr, expected: &str) -> bool {
    matches!(expr, Expr::MethodCall(ExprMethodCall { method, .. }) if method == expected)
}

fn path_last_ident(path: &SynPath) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
