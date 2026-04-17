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
    diag_source_location: Option<&str>,
    mutation_required: bool,
) {
    if matches!(tier, OwnershipTier::RcMutShared | OwnershipTier::Scoped) {
        strip_binding_mutability(&mut local.pat);
    }

    // S-1: PlainOwned local bindings that are mutated need `let mut`.
    if tier == OwnershipTier::PlainOwned && mutation_required {
        add_binding_mutability(&mut local.pat);
    }

    apply_tier_to_pat_type(&mut local.pat, tier);

    let Some(init) = &mut local.init else {
        return;
    };

    match tier {
        OwnershipTier::BoxOwned if !already_wrapped => {
            let expr = (*init.expr).clone();
            init.expr = Box::new(parse_quote!(Box::new(#expr)));
        }
        OwnershipTier::RcShared if !already_wrapped => {
            let expr = (*init.expr).clone();
            init.expr = Box::new(parse_quote!(Rc::new(#expr)));
        }
        OwnershipTier::ArcShared if !already_wrapped => {
            let expr = (*init.expr).clone();
            init.expr = Box::new(parse_quote!(Arc::new(#expr)));
        }
        OwnershipTier::RcMutShared if !already_wrapped => {
            let expr = (*init.expr).clone();
            if let Some(loc) = diag_source_location {
                // DiagOwner wraps the Rc<RefCell<T>> for borrow instrumentation.
                // The source location literal is baked at codegen time — it never
                // allocates at runtime (it is a &'static str).
                init.expr = Box::new(parse_quote!(
                    DiagOwner::new(Rc::new(RefCell::new(#expr)), #loc)
                ));
            } else {
                init.expr = Box::new(parse_quote!(Rc::new(RefCell::new(#expr))));
            }
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

pub(crate) fn binding_tier_from_expr(
    expr: &syn::Expr,
    scopes: &ScopeStack,
) -> Option<(syn::Ident, OwnershipTier)> {
    match expr {
        syn::Expr::Path(path) => binding_tier_from_path(path, scopes),
        syn::Expr::Paren(paren) => binding_tier_from_expr(&paren.expr, scopes),
        syn::Expr::Group(group) => binding_tier_from_expr(&group.expr, scopes),
        _ => None,
    }
}

pub(crate) fn wrapper_binding_from_expr(
    expr: &syn::Expr,
    scopes: &ScopeStack,
) -> Option<(syn::Ident, OwnershipTier)> {
    let (ident, tier) = binding_tier_from_expr(expr, scopes)?;
    matches!(
        tier,
        OwnershipTier::RcShared
            | OwnershipTier::ArcShared
            | OwnershipTier::RcMutShared
            | OwnershipTier::ArcMutShared
    )
    .then_some((ident, tier))
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
        OwnershipTier::BoxOwned => parse_quote!(Box<#original_ty>),
        OwnershipTier::RcShared => parse_quote!(Rc<#original_ty>),
        OwnershipTier::ArcShared => parse_quote!(Arc<#original_ty>),
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

fn add_binding_mutability(pat: &mut syn::Pat) {
    match pat {
        syn::Pat::Ident(ident) => {
            ident.mutability = Some(syn::token::Mut::default());
        }
        syn::Pat::Type(typed) => add_binding_mutability(&mut typed.pat),
        _ => {}
    }
}

fn binding_tier_from_path(
    path: &syn::ExprPath,
    scopes: &ScopeStack,
) -> Option<(syn::Ident, OwnershipTier)> {
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }

    let ident = path.path.segments.first()?.ident.clone();
    let tier = scopes.lookup(&ident)?;
    Some((ident, tier))
}

#[cfg(test)]
mod tests {
    use kobo_ir::OwnershipTier;
    use syn::parse_quote;

    use super::apply_tier_to_local;

    /// Build a `let x: i32 = <init>;` local statement for testing.
    fn make_local(init_expr: syn::Expr) -> syn::Local {
        let stmt: syn::Stmt = parse_quote! { let x: i32 = #init_expr; };
        match stmt {
            syn::Stmt::Local(local) => local,
            _ => panic!("expected Local statement"),
        }
    }

    #[test]
    fn diag_mode_wraps_rc_refcell_in_diag_owner() {
        let init: syn::Expr = parse_quote!(value);
        let mut local = make_local(init);

        apply_tier_to_local(&mut local, OwnershipTier::RcMutShared, false, Some("test.kobo:5"), false);

        let output = quote::quote!(#local).to_string();
        assert!(
            output.contains("DiagOwner"),
            "diag_mode=true must wrap in DiagOwner, got: {output}"
        );
        assert!(
            output.contains("test.kobo:5"),
            "must embed source location literal, got: {output}"
        );
    }

    #[test]
    fn normal_mode_wraps_rc_refcell_without_diag_owner() {
        let init: syn::Expr = parse_quote!(value);
        let mut local = make_local(init);

        apply_tier_to_local(&mut local, OwnershipTier::RcMutShared, false, None, false);

        let output = quote::quote!(#local).to_string();
        assert!(
            !output.contains("DiagOwner"),
            "diag_mode=false must NOT wrap in DiagOwner, got: {output}"
        );
        assert!(
            output.contains("Rc") && output.contains("RefCell"),
            "must wrap in Rc<RefCell<…>>, got: {output}"
        );
    }
}
