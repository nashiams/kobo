use kobo_ir::{FileId, KoboSpan};
use kobo_parser::KoboFile;
use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
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

pub fn collect_error_policy_sites(
    kobo_file: &KoboFile,
    generated_source: &str,
    file_id: FileId,
) -> Vec<ErrorPolicySite> {
    let source_sites = source_try_sites(kobo_file);
    let generated_offsets = generated_try_offsets(generated_source);
    source_sites
        .into_iter()
        .enumerate()
        .filter_map(|(index, source_try)| {
            let generated_offset = generated_offsets.get(index).copied()?;
            let (operation, variant, source_error) =
                classify_try_operation(source_try.expr.as_ref());
            let source_span = kobo_file.span_from_syn(source_try.span());
            Some(ErrorPolicySite {
                source_span: normalized_span(source_span, file_id),
                generated_offset,
                line: one_based_line_for_offset(generated_source, generated_offset),
                operation: operation.to_owned(),
                variant: variant.to_owned(),
                source_error: source_error.to_owned(),
            })
        })
        .collect()
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

fn source_try_sites(kobo_file: &KoboFile) -> Vec<ExprTry> {
    let mut visitor = TrySiteVisitor::default();
    visitor.visit_file(kobo_file.syn_file());
    visitor.sites
}

fn generated_try_offsets(source: &str) -> Vec<usize> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut visitor = TrySiteVisitor::default();
    visitor.visit_file(&file);
    visitor
        .sites
        .into_iter()
        .map(|site| {
            let end = site.span().end();
            line_col_to_offset(source, end.line, end.column)
                .unwrap_or(source.len())
                .saturating_sub(1)
        })
        .collect()
}

#[derive(Default)]
struct TrySiteVisitor {
    sites: Vec<ExprTry>,
}

impl<'ast> Visit<'ast> for TrySiteVisitor {
    fn visit_expr_try(&mut self, node: &'ast ExprTry) {
        self.sites.push(node.clone());
        visit::visit_expr_try(self, node);
    }
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn line_col_to_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut offset = 0usize;
    for (index, text) in source.lines().enumerate() {
        if index + 1 == line {
            return Some((offset + column).min(source.len()));
        }
        offset += text.len() + 1;
    }
    None
}
