use crate::classify::{BindingMetadata, TransformCtx};
use kobo_ir::{EscapeKind, KirNode, NodeKind, OwnershipTier};
use kobo_parser::KoboBinding;

pub(super) fn detect_boxless_resource(
    binding: &KoboBinding,
    init: Option<&syn::Expr>,
    ctx: &TransformCtx,
) -> Option<kobo_ir::ResourceKind> {
    crate::classify::detect_resource_kind(binding.ty.as_ref(), init, ctx)
}

pub(super) fn assignment_escape_kind(target: &syn::Expr) -> Option<EscapeKind> {
    match target {
        syn::Expr::Field(_) => Some(EscapeKind::StoredInStruct),
        syn::Expr::Paren(paren) => assignment_escape_kind(paren.expr.as_ref()),
        syn::Expr::Group(group) => assignment_escape_kind(group.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn build_decl_node(
    binding: &KoboBinding,
    id: kobo_ir::KirNodeId,
    metadata: BindingMetadata,
) -> KirNode {
    KirNode {
        id,
        kind: NodeKind::Decl,
        ast_id: Some(binding.id),
        ownership: OwnershipTier::Undecided,
        resource_kind: metadata.resource_kind,
        cfg_block: None,
        span: binding.span,
        decl_id: None,
    }
}

pub(super) fn build_scope_node(
    id: kobo_ir::KirNodeId,
    kind: NodeKind,
    span: kobo_ir::KoboSpan,
) -> KirNode {
    KirNode {
        id,
        kind,
        ast_id: None,
        ownership: OwnershipTier::PlainOwned,
        resource_kind: None,
        cfg_block: None,
        span,
        decl_id: None,
    }
}

pub(super) fn build_binding_event_node(
    id: kobo_ir::KirNodeId,
    kind: NodeKind,
    span: kobo_ir::KoboSpan,
    decl_id: kobo_ir::KirNodeId,
) -> KirNode {
    KirNode {
        id,
        kind,
        ast_id: None,
        ownership: OwnershipTier::Undecided,
        resource_kind: None,
        cfg_block: None,
        span,
        decl_id: Some(decl_id),
    }
}

pub(super) fn binding_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}

pub(super) fn expr_ident(expr: &syn::Expr) -> Option<&syn::Ident> {
    match expr {
        syn::Expr::Path(path) => {
            if path.qself.is_some() || path.path.segments.len() != 1 {
                return None;
            }

            Some(&path.path.segments.first()?.ident)
        }
        syn::Expr::Paren(paren) => expr_ident(paren.expr.as_ref()),
        syn::Expr::Group(group) => expr_ident(group.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn borrow_kind(reference: &syn::ExprReference) -> kobo_ir::BorrowKind {
    if reference.mutability.is_some() {
        kobo_ir::BorrowKind::Mutable
    } else {
        kobo_ir::BorrowKind::Immutable
    }
}

pub(super) fn is_mutating_method(method: &syn::Ident) -> bool {
    matches!(
        method.to_string().as_str(),
        "append"
            | "clear"
            | "extend"
            | "insert"
            | "insert_str"
            | "pop"
            | "push"
            | "push_str"
            | "remove"
            | "replace"
            | "retain"
            | "reverse"
            | "sort"
            | "sort_by"
            | "swap"
            | "truncate"
    )
}
