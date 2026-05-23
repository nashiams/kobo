use std::collections::{HashMap, HashSet};

use quote::ToTokens;
use syn::visit::{self, Visit};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandlerLeakWarning {
    pub fn_name: String,
    pub binding_name: String,
    pub source_offset: usize,
}

struct HandlerAstContext<'source> {
    source: &'source str,
    warnings: Vec<HandlerLeakWarning>,
}

struct HandlerBodyLeakScan<'context, 'source> {
    context: &'context mut HandlerAstContext<'source>,
    fn_name: String,
    request_bindings: HashSet<String>,
    alias_origins: HashMap<String, String>,
}

pub fn scan_source_handler_leaks(source: &str) -> Vec<HandlerLeakWarning> {
    let mut warnings = ast_handler_leaks(source);
    for warning in text_handler_leaks(source) {
        push_unique_warning(&mut warnings, warning);
    }
    warnings
}

fn ast_handler_leaks(source: &str) -> Vec<HandlerLeakWarning> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut context = HandlerAstContext {
        source,
        warnings: Vec::new(),
    };
    for item in &file.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        if !is_handler_function(function) {
            continue;
        }
        let request_bindings = function_param_names(function);
        if request_bindings.is_empty() {
            continue;
        }
        let mut scan = HandlerBodyLeakScan {
            context: &mut context,
            fn_name: function.sig.ident.to_string(),
            request_bindings,
            alias_origins: HashMap::new(),
        };
        scan.visit_block(&function.block);
    }
    context.warnings
}

fn text_handler_leaks(source: &str) -> Vec<HandlerLeakWarning> {
    let mut warnings = Vec::new();
    for (attr_offset, _) in source.match_indices("#[kobo::handler]") {
        let after_attr = &source[attr_offset..];
        let Some(async_rel) = after_attr.find("async fn ") else {
            continue;
        };
        let sig_offset = attr_offset + async_rel;
        let sig = &source[sig_offset..];
        let Some(open_paren_rel) = sig.find('(') else {
            continue;
        };
        let name = sig["async fn ".len()..open_paren_rel].trim();
        let open_paren = sig_offset + open_paren_rel;
        let Some(close_paren) = find_matching_delimiter(source, open_paren, '(', ')') else {
            continue;
        };
        let params = extract_param_names(&source[open_paren + 1..close_paren]);
        if params.is_empty() {
            continue;
        }
        let Some(body_open_rel) = source[close_paren..].find('{') else {
            continue;
        };
        let body_open = close_paren + body_open_rel;
        let Some(body_close) = find_matching_delimiter(source, body_open, '{', '}') else {
            continue;
        };
        let body = &source[body_open + 1..body_close];
        for param in &params {
            if let Some(leak_rel) = find_spawn_capture(body, param) {
                warnings.push(HandlerLeakWarning {
                    fn_name: name.to_owned(),
                    binding_name: param.clone(),
                    source_offset: body_open + 1 + leak_rel,
                });
            }
        }
    }
    warnings
}

impl HandlerBodyLeakScan<'_, '_> {
    fn origin_for_ident(&self, ident: &str) -> Option<String> {
        if self.request_bindings.contains(ident) {
            return Some(ident.to_owned());
        }
        self.alias_origins.get(ident).cloned()
    }

    fn origin_for_expr(&self, expr: &syn::Expr) -> Option<String> {
        root_ident_of_expr(expr).and_then(|ident| self.origin_for_ident(&ident))
    }

    fn push_warning(&mut self, binding_name: String) {
        let source_offset = self.context.source.find(&binding_name).unwrap_or_default();
        push_unique_warning(
            &mut self.context.warnings,
            HandlerLeakWarning {
                fn_name: self.fn_name.clone(),
                binding_name,
                source_offset,
            },
        );
    }

    fn push_if_expr_escapes(&mut self, expr: &syn::Expr) {
        if let Some(binding_name) = self.origin_for_expr(expr) {
            self.push_warning(binding_name);
        }
    }

    fn push_if_expr_contains_request(&mut self, expr: &syn::Expr) {
        for binding_name in self.request_bindings.clone() {
            if expr_contains_binding(expr, &binding_name) {
                self.push_warning(binding_name);
            }
        }
        for (alias, binding_name) in self.alias_origins.clone() {
            if expr_contains_binding(expr, &alias) {
                self.push_warning(binding_name);
            }
        }
    }
}

impl<'ast> Visit<'ast> for HandlerBodyLeakScan<'_, '_> {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some(local_ident) = ident_from_pat(&node.pat) {
            if let Some(init) = &node.init {
                if let Some(origin) = self.origin_for_expr(&init.expr) {
                    self.alias_origins.insert(local_ident, origin);
                }
            }
        }
        visit::visit_local(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        let function_name = call_function_name(&node.func);
        if function_name.as_deref().is_some_and(is_spawn_call_name) {
            for arg in &node.args {
                self.push_if_expr_contains_request(arg);
            }
        } else if function_name.as_deref().is_some_and(is_sink_function_name) {
            for arg in &node.args {
                self.push_if_expr_escapes(arg);
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method_name = node.method.to_string();
        if is_sink_method_name(&method_name) {
            for arg in &node.args {
                self.push_if_expr_escapes(arg);
            }
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn push_unique_warning(warnings: &mut Vec<HandlerLeakWarning>, warning: HandlerLeakWarning) {
    if warnings.iter().any(|existing| {
        existing.fn_name == warning.fn_name && existing.binding_name == warning.binding_name
    }) {
        return;
    }
    warnings.push(warning);
}

fn is_handler_function(function: &syn::ItemFn) -> bool {
    function.sig.asyncness.is_some()
        && function.attrs.iter().any(|attr| {
            let segments = attr.path().segments.iter().collect::<Vec<_>>();
            segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "handler"
        })
}

fn function_param_names(function: &syn::ItemFn) -> HashSet<String> {
    function
        .sig
        .inputs
        .iter()
        .filter_map(|input| {
            let syn::FnArg::Typed(argument) = input else {
                return None;
            };
            ident_from_pat(&argument.pat)
        })
        .collect()
}

fn ident_from_pat(pat: &syn::Pat) -> Option<String> {
    let syn::Pat::Ident(pat_ident) = pat else {
        return None;
    };
    Some(pat_ident.ident.to_string())
}

fn root_ident_of_expr(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) => path
            .path
            .segments
            .first()
            .map(|segment| segment.ident.to_string()),
        syn::Expr::Field(field) => root_ident_of_expr(&field.base),
        syn::Expr::Reference(reference) => root_ident_of_expr(&reference.expr),
        syn::Expr::Paren(paren) => root_ident_of_expr(&paren.expr),
        _ => None,
    }
}

fn call_function_name(func: &syn::Expr) -> Option<String> {
    Some(func.to_token_stream().to_string())
}

fn is_spawn_call_name(name: &str) -> bool {
    name.split_whitespace()
        .collect::<String>()
        .contains("spawn")
}

fn is_sink_function_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("store")
        || lower.contains("sink")
        || lower.contains("background")
        || lower.contains("later")
}

fn is_sink_method_name(name: &str) -> bool {
    matches!(name, "send" | "try_send" | "push" | "insert")
}

fn expr_contains_binding(expr: &syn::Expr, binding: &str) -> bool {
    let mut visitor = BindingUseVisitor {
        binding,
        contains_binding: false,
    };
    match expr {
        syn::Expr::Async(async_expr) => visitor.visit_block(&async_expr.block),
        syn::Expr::Closure(closure) => visitor.visit_expr(&closure.body),
        _ => visitor.visit_expr(expr),
    }
    visitor.contains_binding
}

struct BindingUseVisitor<'binding> {
    binding: &'binding str,
    contains_binding: bool,
}

impl<'ast> Visit<'ast> for BindingUseVisitor<'_> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == self.binding)
        {
            self.contains_binding = true;
        }
        visit::visit_expr_path(self, node);
    }
}

fn extract_param_names(params: &str) -> Vec<String> {
    params
        .split(',')
        .filter_map(|param| {
            let name = param.split(':').next()?.trim();
            let name = name
                .trim_start_matches("mut ")
                .trim_start_matches('&')
                .trim_start_matches("mut ")
                .trim();
            if name.is_empty() || name == "self" {
                None
            } else {
                Some(name.to_owned())
            }
        })
        .collect()
}

fn find_spawn_capture(body: &str, binding: &str) -> Option<usize> {
    let mut escaped_bindings = vec![binding.to_owned()];
    escaped_bindings.extend(request_aliases(body, binding));
    for marker in ["spawn {", "tokio::spawn", "spawn_local"] {
        let mut search_offset = 0usize;
        while let Some(found) = body[search_offset..].find(marker) {
            let spawn_pos = search_offset + found;
            let spawned_body = spawn_extent(body, spawn_pos, marker)
                .map(|(start, end)| &body[start..end])
                .unwrap_or(&body[spawn_pos..]);
            if escaped_bindings
                .iter()
                .any(|escaped_binding| contains_ident(spawned_body, escaped_binding))
            {
                return Some(spawn_pos);
            }
            search_offset = spawn_pos + marker.len();
        }
    }
    None
}

fn request_aliases(body: &str, binding: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let alias = trimmed.strip_prefix("let ")?.split('=').next()?.trim();
            let assigned = trimmed
                .split('=')
                .nth(1)?
                .trim()
                .trim_end_matches(';')
                .trim();
            (assigned == binding).then_some(alias.to_owned())
        })
        .collect()
}

fn spawn_extent(body: &str, spawn_pos: usize, marker: &str) -> Option<(usize, usize)> {
    if marker == "spawn {" {
        let brace_pos = body[spawn_pos..].find('{')? + spawn_pos;
        let close = find_matching_delimiter(body, brace_pos, '{', '}')?;
        return Some((spawn_pos, close + 1));
    }

    let after_marker = spawn_pos + marker.len();
    let delimiter_rel = body[after_marker..]
        .char_indices()
        .find(|(_, ch)| *ch == '(' || *ch == '{')?;
    let delimiter_pos = after_marker + delimiter_rel.0;
    let delimiter = delimiter_rel.1;
    let close = match delimiter {
        '(' => find_matching_delimiter(body, delimiter_pos, '(', ')')?,
        '{' => find_matching_delimiter(body, delimiter_pos, '{', '}')?,
        _ => return None,
    };
    Some((spawn_pos, close + 1))
}

fn contains_ident(source: &str, ident: &str) -> bool {
    source.match_indices(ident).any(|(idx, _)| {
        let before = source[..idx].chars().next_back();
        let after = source[idx + ident.len()..].chars().next();
        !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
    })
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn find_matching_delimiter(
    source: &str,
    open_offset: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    for (rel, ch) in source[open_offset..].char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(open_offset + rel);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_request_capture_in_spawn() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    spawn {
        println!("{}", req.path);
    }
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].fn_name, "handle");
        assert_eq!(warnings[0].binding_name, "req");
    }

    #[test]
    fn ignores_handler_without_spawn_capture() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    println!("{}", req.path);
    spawn {
        println!("background");
    }
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert!(warnings.is_empty(), "warnings: {warnings:?}");
    }

    #[test]
    fn detects_capture_in_later_spawn() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    spawn {
        println!("background");
    }
    tokio::spawn(async move {
        println!("{}", req.path);
    });
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].binding_name, "req");
    }

    #[test]
    fn ignores_binding_after_spawn_expression() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    tokio::spawn(async move {
        println!("background");
    });
    println!("{}", req.path);
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert!(warnings.is_empty(), "warnings: {warnings:?}");
    }

    #[test]
    fn detects_request_alias_capture_in_spawn() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    let request_for_task = req;
    tokio::spawn(async move {
        println!("{}", request_for_task.path);
    });
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].fn_name, "handle");
        assert_eq!(warnings[0].binding_name, "req");
    }

    #[test]
    fn detects_request_escape_into_sink_helper() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    store_request_for_later(req);
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].fn_name, "handle");
        assert_eq!(warnings[0].binding_name, "req");
    }
}
