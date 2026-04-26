use std::collections::HashMap;

use kobo_ir::OwnershipTier;
use syn::parse_quote;

use super::super::binding::is_mutating_method;

/// Remove all `#[kobo::...]` attributes from an attribute list.
pub(super) fn strip_kobo_attrs(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|attr| {
        attr.path()
            .segments
            .first()
            .is_none_or(|segment| segment.ident != "kobo")
    });
}

/// Build a `borrow()` or `borrow_mut()` receiver expression based on whether
/// the method being called is a mutating method.
///
/// Checks the KIR-level `method_mutability` map first (from impl-block scanning
/// + config overrides), then falls back to the hardcoded list.
///
/// When `receiver_type` is provided, a qualified lookup (`"Type::method"`) is
/// attempted before the bare-name fallback, so that same-named methods on
/// different types (e.g. `Reader::touch` vs `Writer::touch`) are resolved
/// correctly (BUG 3/27).
pub(super) fn lowered_receiver_expr(
    ident: syn::Ident,
    method: &syn::Ident,
    method_mutability: &HashMap<String, bool>,
    receiver_type: Option<&str>,
) -> syn::Expr {
    let is_mut = receiver_is_mutating(method, method_mutability, receiver_type);
    if is_mut {
        parse_quote!(#ident.borrow_mut())
    } else {
        parse_quote!(#ident.borrow())
    }
}

pub(super) fn lowered_async_receiver_expr(
    ident: syn::Ident,
    method: &syn::Ident,
    method_mutability: &HashMap<String, bool>,
    receiver_type: Option<&str>,
    in_async_context: bool,
) -> syn::Expr {
    let is_mut = receiver_is_mutating(method, method_mutability, receiver_type);
    match (is_mut, in_async_context) {
        (true, true) => parse_quote!(#ident.write().await),
        (false, true) => parse_quote!(#ident.read().await),
        (true, false) => parse_quote!(#ident.blocking_write()),
        (false, false) => parse_quote!(#ident.blocking_read()),
    }
}

fn receiver_is_mutating(
    method: &syn::Ident,
    method_mutability: &HashMap<String, bool>,
    receiver_type: Option<&str>,
) -> bool {
    let method_name = method.to_string();
    receiver_type
        .and_then(|ty| {
            let qkey = format!("{ty}::{method_name}");
            method_mutability.get(&qkey).copied()
        })
        .or_else(|| method_mutability.get(&method_name).copied())
        .unwrap_or_else(|| is_mutating_method(method))
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

    use super::{lowered_async_receiver_expr, lowered_receiver_expr};

    fn render_expr(expr: &syn::Expr) -> String {
        expr.to_token_stream().to_string()
    }

    #[test]
    fn registry_marked_mutating_method_uses_borrow_mut() {
        let mut method_mutability = HashMap::new();
        method_mutability.insert("flush".to_owned(), true);

        let method: syn::Ident = parse_quote!(flush);
        let expr = lowered_receiver_expr(parse_quote!(logger), &method, &method_mutability, None);

        assert_eq!(render_expr(&expr), "logger . borrow_mut ()");
    }

    #[test]
    fn standard_immutable_method_uses_borrow() {
        let method_mutability = HashMap::new();
        let method: syn::Ident = parse_quote!(len);
        let expr = lowered_receiver_expr(parse_quote!(logger), &method, &method_mutability, None);

        assert_eq!(render_expr(&expr), "logger . borrow ()");
    }

    #[test]
    fn unknown_methods_default_to_borrow_mut_per_contract() {
        let method_mutability = HashMap::new();
        let method: syn::Ident = parse_quote!(flush_cache);
        let expr = lowered_receiver_expr(parse_quote!(logger), &method, &method_mutability, None);

        assert_eq!(render_expr(&expr), "logger . borrow_mut ()");
    }

    #[test]
    fn qualified_lookup_overrides_bare_conservative() {
        let mut method_mutability = HashMap::new();
        // bare "touch" is conservative (true) because Writer::touch is &mut self
        method_mutability.insert("touch".to_owned(), true);
        // but Reader::touch is specifically &self
        method_mutability.insert("Reader::touch".to_owned(), false);

        let method: syn::Ident = parse_quote!(touch);

        // With receiver type "Reader", should use the qualified immutable answer
        let expr =
            lowered_receiver_expr(parse_quote!(r), &method, &method_mutability, Some("Reader"));
        assert_eq!(render_expr(&expr), "r . borrow ()");

        // Without receiver type, falls back to bare conservative (mutating)
        let expr2 = lowered_receiver_expr(parse_quote!(r), &method, &method_mutability, None);
        assert_eq!(render_expr(&expr2), "r . borrow_mut ()");
    }

    #[test]
    fn async_rwlock_receiver_uses_write_await_for_mutating_methods() {
        let mut method_mutability = HashMap::new();
        method_mutability.insert("Writer::push".to_owned(), true);

        let method: syn::Ident = parse_quote!(push);
        let expr = lowered_async_receiver_expr(
            parse_quote!(data),
            &method,
            &method_mutability,
            Some("Writer"),
            true,
        );

        assert_eq!(render_expr(&expr), "data . write () . await");
    }

    #[test]
    fn sync_rwlock_receiver_uses_blocking_read_for_immutable_methods() {
        let mut method_mutability = HashMap::new();
        method_mutability.insert("Reader::len".to_owned(), false);

        let method: syn::Ident = parse_quote!(len);
        let expr = lowered_async_receiver_expr(
            parse_quote!(data),
            &method,
            &method_mutability,
            Some("Reader"),
            false,
        );

        assert_eq!(render_expr(&expr), "data . blocking_read ()");
    }
}
