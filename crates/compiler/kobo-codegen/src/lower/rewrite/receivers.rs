use quote::ToTokens;

struct MutationVisitor {
    source: String,
    found: bool,
}

pub(super) fn iterator_source_ident(expr: &syn::Expr) -> Option<syn::Ident> {
    let syn::Expr::MethodCall(method_call) = expr else {
        return None;
    };
    if method_call.method != "iter" || !method_call.args.is_empty() {
        return None;
    }
    let syn::Expr::Path(path) = method_call.receiver.as_ref() else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    Some(path.path.segments.first()?.ident.clone())
}

pub(super) fn block_mutates_binding(block: &syn::Block, source_ident: &syn::Ident) -> bool {
    block_mutates_binding_name(block, &source_ident.to_string())
}

pub(super) fn block_mutates_binding_name(block: &syn::Block, source: &str) -> bool {
    let mut visitor = MutationVisitor {
        source: source.to_owned(),
        found: false,
    };
    syn::visit::Visit::visit_block(&mut visitor, block);
    visitor.found
}

pub(super) fn block_mentions_ward_boundary(block: &syn::Block) -> bool {
    block.to_token_stream().to_string().contains("ward")
}

impl<'ast> syn::visit::Visit<'ast> for MutationVisitor {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if receiver_matches_source(node.receiver.as_ref(), &self.source)
            && mutating_collection_method(&node.method)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if receiver_matches_source(node.left.as_ref(), &self.source) {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_assign(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        if is_compound_assignment(&node.op)
            && receiver_matches_source(node.left.as_ref(), &self.source)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_binary(self, node);
    }
}

fn receiver_matches_source(expr: &syn::Expr, source: &str) -> bool {
    match expr {
        syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => path
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == source),
        syn::Expr::Field(field) => receiver_matches_source(field.base.as_ref(), source),
        syn::Expr::Index(index) => receiver_matches_source(index.expr.as_ref(), source),
        syn::Expr::Paren(paren) => receiver_matches_source(paren.expr.as_ref(), source),
        syn::Expr::Group(group) => receiver_matches_source(group.expr.as_ref(), source),
        _ => false,
    }
}

fn mutating_collection_method(method: &syn::Ident) -> bool {
    matches!(
        method.to_string().as_str(),
        "push"
            | "pop"
            | "insert"
            | "remove"
            | "clear"
            | "extend"
            | "retain"
            | "resize"
            | "truncate"
            | "swap_remove"
    )
}

fn is_compound_assignment(op: &syn::BinOp) -> bool {
    matches!(
        op,
        syn::BinOp::AddAssign(_)
            | syn::BinOp::SubAssign(_)
            | syn::BinOp::MulAssign(_)
            | syn::BinOp::DivAssign(_)
            | syn::BinOp::RemAssign(_)
            | syn::BinOp::BitXorAssign(_)
            | syn::BinOp::BitAndAssign(_)
            | syn::BinOp::BitOrAssign(_)
            | syn::BinOp::ShlAssign(_)
            | syn::BinOp::ShrAssign(_)
    )
}
