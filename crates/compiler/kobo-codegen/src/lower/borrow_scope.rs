use kobo_ir::OwnershipTier;
use syn::visit::{self, Visit};

use super::scope::ScopeStack;

pub(crate) struct BorrowScopeAlias {
    pub alias_ident: syn::Ident,
    pub source_ident: syn::Ident,
    pub is_mutable: bool,
}

pub(crate) fn simple_borrow_alias(
    local: &syn::Local,
    scopes: &ScopeStack,
) -> Option<BorrowScopeAlias> {
    let syn::Pat::Ident(alias_pat) = &local.pat else {
        return None;
    };
    let init = local.init.as_ref()?;
    let syn::Expr::Reference(reference) = init.expr.as_ref() else {
        return None;
    };
    let syn::Expr::Path(path) = reference.expr.as_ref() else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }

    let source_ident = path.path.segments.first()?.ident.clone();
    let source_tier = scopes.lookup(&source_ident)?;
    if source_tier != OwnershipTier::RcMutShared {
        return None;
    }

    Some(BorrowScopeAlias {
        alias_ident: alias_pat.ident.clone(),
        source_ident,
        is_mutable: reference.mutability.is_some(),
    })
}

pub(crate) fn has_later_alias_use(statements: &[syn::Stmt], alias: &syn::Ident) -> bool {
    statements
        .iter()
        .any(|statement| ident_use_count_in_stmt(statement, alias) > 0)
}

pub(crate) fn rewritable_method_call(
    statement: &syn::Stmt,
    alias: &syn::Ident,
) -> Option<(syn::ExprMethodCall, bool)> {
    if has_scope_boundary(statement) || ident_use_count_in_stmt(statement, alias) != 1 {
        return None;
    }

    let syn::Stmt::Expr(expr, semi) = statement else {
        return None;
    };
    let syn::Expr::MethodCall(method_call) = expr else {
        return None;
    };
    let syn::Expr::Path(path) = method_call.receiver.as_ref() else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    if path.path.segments.first()?.ident != *alias {
        return None;
    }

    Some((method_call.clone(), semi.is_some()))
}

fn has_scope_boundary(statement: &syn::Stmt) -> bool {
    let mut visitor = ScopeBoundaryVisitor::default();
    visitor.visit_stmt(statement);
    visitor.has_boundary
}

fn ident_use_count_in_stmt(statement: &syn::Stmt, alias: &syn::Ident) -> usize {
    let mut visitor = IdentUseCounter::new(alias);
    visitor.visit_stmt(statement);
    visitor.count
}

#[derive(Default)]
struct ScopeBoundaryVisitor {
    has_boundary: bool,
}

impl<'ast> Visit<'ast> for ScopeBoundaryVisitor {
    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.has_boundary = true;
        visit::visit_expr_closure(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.has_boundary = true;
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        self.has_boundary = true;
        visit::visit_expr_if(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.has_boundary = true;
        visit::visit_expr_loop(self, node);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        self.has_boundary = true;
        visit::visit_expr_match(self, node);
    }

    fn visit_expr_return(&mut self, node: &'ast syn::ExprReturn) {
        self.has_boundary = true;
        visit::visit_expr_return(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.has_boundary = true;
        visit::visit_expr_try(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.has_boundary = true;
        visit::visit_expr_while(self, node);
    }
}

struct IdentUseCounter<'a> {
    alias: &'a syn::Ident,
    count: usize,
}

impl<'a> IdentUseCounter<'a> {
    fn new(alias: &'a syn::Ident) -> Self {
        Self { alias, count: 0 }
    }
}

impl<'ast> Visit<'ast> for IdentUseCounter<'_> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.qself.is_none()
            && node.path.segments.len() == 1
            && node
                .path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == *self.alias)
        {
            self.count += 1;
        }
        visit::visit_expr_path(self, node);
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::has_scope_boundary;

    #[test]
    fn if_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(if cond {
            alias.push(1);
        });
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn match_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(match alias.len() {
            _ => alias.push(1),
        });
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn loop_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(loop {
            alias.push(1);
        });
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn while_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(while cond {
            alias.push(1);
        });
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn for_loop_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(for value in alias.iter() {
            println!("{}", value);
        });
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn closures_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!((|| alias.push(1))(););
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn try_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(alias.try_push(1)?;);
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn return_expressions_are_scope_boundaries() {
        let statement: syn::Stmt = parse_quote!(return alias.len(););
        assert!(has_scope_boundary(&statement));
    }

    #[test]
    fn simple_method_calls_can_still_shrink() {
        let statement: syn::Stmt = parse_quote!(alias.push(1););
        assert!(!has_scope_boundary(&statement));
    }
}
