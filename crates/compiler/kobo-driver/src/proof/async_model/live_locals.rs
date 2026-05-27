use std::collections::BTreeSet;

use kobo_ir::FileId;
use kobo_parser::{
    preprocess_bridge_blocks, preprocess_concurrent_sugar, preprocess_kobo_keywords,
    preprocess_spawn_blocks, strict_keyword_configs,
};

mod expr_traversal;

use expr_traversal::await_live_locals_in_stmt;

struct AwaitVisitor {
    count: usize,
}

struct StmtUseVisitor<'a> {
    uses: &'a mut BTreeSet<String>,
}

struct ExprUseVisitor<'a> {
    uses: &'a mut BTreeSet<String>,
}

impl<'ast> syn::visit::Visit<'ast> for AwaitVisitor {
    fn visit_expr_await(&mut self, expr: &'ast syn::ExprAwait) {
        self.count += 1;
        syn::visit::visit_expr_await(self, expr);
    }
}

impl<'a, 'ast> syn::visit::Visit<'ast> for StmtUseVisitor<'a> {
    fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
        if expr.qself.is_none() && expr.path.segments.len() == 1 {
            if let Some(segment) = expr.path.segments.first() {
                self.uses.insert(segment.ident.to_string());
            }
        }
        syn::visit::visit_expr_path(self, expr);
    }
}

impl<'a, 'ast> syn::visit::Visit<'ast> for ExprUseVisitor<'a> {
    fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
        if expr.qself.is_none() && expr.path.segments.len() == 1 {
            if let Some(segment) = expr.path.segments.first() {
                self.uses.insert(segment.ident.to_string());
            }
        }
        syn::visit::visit_expr_path(self, expr);
    }
}

pub(super) fn parsed_live_locals_by_await(source: &str, target: &str) -> Vec<Vec<String>> {
    let Some(file) = parse_source_for_async_model(source) else {
        return Vec::new();
    };
    let Some(function) = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == target => Some(function),
        _ => None,
    }) else {
        return Vec::new();
    };

    let mut locals_before_statement = Vec::<BTreeSet<String>>::new();
    let mut declared = BTreeSet::<String>::new();
    for statement in &function.block.stmts {
        locals_before_statement.push(declared.clone());
        collect_pat_bindings_in_stmt(statement, &mut declared);
    }

    let mut later_statement_uses = vec![BTreeSet::<String>::new(); function.block.stmts.len()];
    let mut suffix_uses = BTreeSet::<String>::new();
    for (index, statement) in function.block.stmts.iter().enumerate().rev() {
        later_statement_uses[index] = suffix_uses.clone();
        collect_ident_uses_in_stmt(statement, &mut suffix_uses);
    }

    function
        .block
        .stmts
        .iter()
        .enumerate()
        .flat_map(|(index, statement)| {
            let declared_before = locals_before_statement
                .get(index)
                .cloned()
                .unwrap_or_default();
            await_live_locals_in_stmt(statement, &later_statement_uses[index], &declared_before)
                .into_iter()
                .map(|locals| locals.into_iter().collect::<Vec<_>>())
        })
        .collect()
}

fn parse_source_for_async_model(source: &str) -> Option<syn::File> {
    syn::parse_file(source)
        .or_else(|_| syn::parse_file(&preprocess_kobo_source_for_syn(source)))
        .ok()
}

fn preprocess_kobo_source_for_syn(source: &str) -> String {
    let (rewritten, _) = preprocess_concurrent_sugar(source);
    let configs = strict_keyword_configs();
    let (rewritten, _) = preprocess_kobo_keywords(&rewritten, &configs);
    let (rewritten, _) = preprocess_spawn_blocks(&rewritten, FileId(0));
    let (rewritten, _) = preprocess_bridge_blocks(&rewritten);
    rewritten
}

pub(super) fn ident_uses_in_expr(expr: &syn::Expr) -> BTreeSet<String> {
    let mut uses = BTreeSet::new();
    collect_ident_uses_in_expr(expr, &mut uses);
    uses
}

pub(super) fn expr_await_count(expr: &syn::Expr) -> usize {
    let mut visitor = AwaitVisitor { count: 0 };
    syn::visit::visit_expr(&mut visitor, expr);
    visitor.count
}

pub(super) fn collect_pat_bindings_in_stmt(statement: &syn::Stmt, bindings: &mut BTreeSet<String>) {
    let syn::Stmt::Local(local) = statement else {
        return;
    };
    collect_pat_bindings(&local.pat, bindings);
}

pub(super) fn collect_pat_bindings(pattern: &syn::Pat, bindings: &mut BTreeSet<String>) {
    match pattern {
        syn::Pat::Ident(ident) => {
            bindings.insert(ident.ident.to_string());
        }
        syn::Pat::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Struct(pattern) => {
            for field in &pattern.fields {
                collect_pat_bindings(&field.pat, bindings);
            }
        }
        syn::Pat::TupleStruct(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Slice(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Reference(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Type(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Or(pattern) => {
            for case in &pattern.cases {
                collect_pat_bindings(case, bindings);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_ident_uses_in_stmt(statement: &syn::Stmt, uses: &mut BTreeSet<String>) {
    let mut visitor = StmtUseVisitor { uses };
    syn::visit::visit_stmt(&mut visitor, statement);
}

pub(super) fn collect_ident_uses_in_expr(expr: &syn::Expr, uses: &mut BTreeSet<String>) {
    let mut visitor = ExprUseVisitor { uses };
    syn::visit::visit_expr(&mut visitor, expr);
}
