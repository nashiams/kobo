use kobo_ir::OwnershipTier;
use kobo_parser::{KoboBinding, KoboFile};
use syn::parse_quote;

use super::scope::ScopeStack;

pub(crate) fn binding_for_pat<'a>(ast: &'a KoboFile, pat: &syn::Pat) -> Option<&'a KoboBinding> {
    let ident = binding_ident(pat)?;
    let span = ast.span_from_syn(ident.span());
    ast.binding_for_span(span)
}

pub(crate) fn apply_tier_to_local(
    local: &mut syn::Local,
    tier: OwnershipTier,
    already_wrapped: bool,
) {
    if matches!(tier, OwnershipTier::RcMutShared | OwnershipTier::Scoped) {
        strip_binding_mutability(&mut local.pat);
    }

    apply_tier_to_pat_type(&mut local.pat, tier);

    let Some(init) = &mut local.init else {
        return;
    };

    match tier {
        OwnershipTier::RcMutShared if !already_wrapped => {
            let expr = (*init.expr).clone();
            init.expr = Box::new(parse_quote!(Rc::new(RefCell::new(#expr))));
        }
        OwnershipTier::Scoped => {
            let expr = (*init.expr).clone();
            init.expr = Box::new(parse_quote!(ScopedHandle::new(#expr)));
        }
        _ => {}
    }
}

pub(crate) fn apply_tier_to_fn_arg_type(argument: &mut syn::PatType, tier: OwnershipTier) {
    let original_ty = (*argument.ty).clone();
    argument.ty = Box::new(wrap_owned_type(original_ty, tier));
}

pub(crate) fn wrapped_ident_from_expr(expr: &syn::Expr, scopes: &ScopeStack) -> Option<syn::Ident> {
    match expr {
        syn::Expr::Path(path) => wrapped_ident_from_path(path, scopes),
        syn::Expr::Paren(paren) => wrapped_ident_from_expr(&paren.expr, scopes),
        syn::Expr::Group(group) => wrapped_ident_from_expr(&group.expr, scopes),
        _ => None,
    }
}

pub(crate) fn is_mutating_method(method: &syn::Ident) -> bool {
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

fn binding_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}

fn apply_tier_to_pat_type(pat: &mut syn::Pat, tier: OwnershipTier) {
    let syn::Pat::Type(typed) = pat else {
        return;
    };

    let original_ty = (*typed.ty).clone();
    typed.ty = Box::new(wrap_owned_type(original_ty, tier));
}

fn wrap_owned_type(original_ty: syn::Type, tier: OwnershipTier) -> syn::Type {
    match tier {
        OwnershipTier::RcMutShared => parse_quote!(Rc<RefCell<#original_ty>>),
        OwnershipTier::Scoped => parse_quote!(ScopedHandle<#original_ty>),
        _ => original_ty,
    }
}

fn strip_binding_mutability(pat: &mut syn::Pat) {
    match pat {
        syn::Pat::Ident(ident) => ident.mutability = None,
        syn::Pat::Type(typed) => strip_binding_mutability(&mut typed.pat),
        _ => {}
    }
}

fn wrapped_ident_from_path(path: &syn::ExprPath, scopes: &ScopeStack) -> Option<syn::Ident> {
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }

    let ident = path.path.segments.first()?.ident.clone();
    (scopes.lookup(&ident) == Some(OwnershipTier::RcMutShared)).then_some(ident)
}
