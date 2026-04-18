use std::collections::HashMap;

use kobo_ir::OwnershipTier;
use syn::parse_quote;

use super::super::binding::is_mutating_method;

/// Remove all `#[kobo::...]` attributes from an attribute list.
pub(super) fn strip_kobo_attrs(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|attr| {
        !attr
            .path()
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "kobo")
    });
}

/// Build a `borrow()` or `borrow_mut()` receiver expression based on whether
/// the method being called is a mutating method.
///
/// Checks the KIR-level `method_mutability` map first (from impl-block scanning
/// + config overrides), then falls back to the hardcoded list.
pub(super) fn lowered_receiver_expr(
    ident: syn::Ident,
    method: &syn::Ident,
    method_mutability: &HashMap<String, bool>,
) -> syn::Expr {
    let is_mut = method_mutability
        .get(&method.to_string())
        .copied()
        .unwrap_or_else(|| is_mutating_method(method));
    if is_mut {
        parse_quote!(#ident.borrow_mut())
    } else {
        parse_quote!(#ident.borrow())
    }
}

/// Return true if passing an argument of the given tier requires wrapping it
/// (e.g. `Box::new(...)`, `Rc::new(...)`, etc.).
pub(super) fn should_wrap_argument(tier: OwnershipTier) -> bool {
    matches!(
        tier,
        OwnershipTier::BoxOwned
            | OwnershipTier::RcShared
            | OwnershipTier::ArcShared
            | OwnershipTier::RcMutShared
            | OwnershipTier::Scoped
    )
}

/// Wrap an argument expression for the given ownership tier.
pub(super) fn wrap_argument_expr(expr: syn::Expr, tier: OwnershipTier) -> syn::Expr {
    match tier {
        OwnershipTier::BoxOwned => parse_quote!(Box::new(#expr)),
        OwnershipTier::RcShared => parse_quote!(Rc::new(#expr)),
        OwnershipTier::ArcShared => parse_quote!(Arc::new(#expr)),
        OwnershipTier::RcMutShared => parse_quote!(Rc::new(RefCell::new(#expr))),
        OwnershipTier::Scoped => parse_quote!(ScopedHandle::new(#expr)),
        _ => expr,
    }
}

/// Wrap a method call expression in a block statement, optionally with a
/// trailing semicolon.
pub(super) fn build_borrow_scope_block_stmt(
    method_call: syn::ExprMethodCall,
    had_semi: bool,
) -> syn::Stmt {
    let method_expr = syn::Expr::MethodCall(method_call);
    if had_semi {
        parse_quote!({
            #method_expr;
        })
    } else {
        parse_quote!({
            #method_expr
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use quote::ToTokens;
    use syn::parse_quote;

    use super::lowered_receiver_expr;

    fn render_expr(expr: &syn::Expr) -> String {
        expr.to_token_stream().to_string()
    }

    #[test]
    fn registry_marked_mutating_method_uses_borrow_mut() {
        let mut method_mutability = HashMap::new();
        method_mutability.insert("flush".to_owned(), true);

        let method: syn::Ident = parse_quote!(flush);
        let expr = lowered_receiver_expr(parse_quote!(logger), &method, &method_mutability);

        assert_eq!(render_expr(&expr), "logger . borrow_mut ()");
    }

    #[test]
    fn standard_immutable_method_uses_borrow() {
        let method_mutability = HashMap::new();
        let method: syn::Ident = parse_quote!(len);
        let expr = lowered_receiver_expr(parse_quote!(logger), &method, &method_mutability);

        assert_eq!(render_expr(&expr), "logger . borrow ()");
    }

    #[test]
    fn unknown_methods_default_to_borrow_mut_per_contract() {
        let method_mutability = HashMap::new();
        let method: syn::Ident = parse_quote!(flush_cache);
        let expr = lowered_receiver_expr(parse_quote!(logger), &method, &method_mutability);

        assert_eq!(render_expr(&expr), "logger . borrow_mut ()");
    }
}
