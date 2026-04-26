/// Lower `__kobo_spawn_block!({ body })` to `tokio::spawn(async move { body })`.
///
/// The preprocessor rewrites `spawn { ... }` → `__kobo_spawn_block!({ ... })`.
/// After syn parsing, this module recognizes the macro and replaces it with
/// the actual `tokio::spawn(async move { ... })` call.
///
/// S-53: When `use_spawn_local` is true, emits `tokio::task::spawn_local`
/// instead of `tokio::spawn` for non-Send captured bindings.
///
/// Clone injection (Phase 3) will insert `.clone()` calls for shared bindings
/// before the `tokio::spawn` call. This phase only generates the spawn structure.

use syn::parse_quote;

/// Name of the marker macro emitted by the spawn preprocessor.
const SPAWN_MACRO_NAME: &str = "__kobo_spawn_block";

/// Check if a macro invocation is a spawn block marker.
pub(crate) fn is_spawn_block_macro(mac: &syn::Macro) -> bool {
    mac.path
        .get_ident()
        .map(|id| id == SPAWN_MACRO_NAME)
        .unwrap_or(false)
}

/// Lower a `__kobo_spawn_block!({ body })` macro into `tokio::spawn(async move { body })`.
///
/// Returns `Some(expr)` if the macro was a spawn block, `None` otherwise.
pub(crate) fn lower_spawn_macro(mac: &syn::Macro) -> Option<syn::Expr> {
    lower_spawn_macro_with_strategy(mac, false)
}

/// S-53: Lower spawn block with LocalSet strategy selection.
///
/// When `use_spawn_local` is true, generates `tokio::task::spawn_local(async move { body })`
/// instead of `tokio::spawn(async move { body })`.
pub(crate) fn lower_spawn_macro_with_strategy(mac: &syn::Macro, use_spawn_local: bool) -> Option<syn::Expr> {
    if !is_spawn_block_macro(mac) {
        return None;
    }

    // Parse the macro body as a Block (the preprocessor wraps in { ... }).
    let block: syn::Block = syn::parse2(mac.tokens.clone()).ok()?;

    let stmts = &block.stmts;
    if use_spawn_local {
        Some(parse_quote! {
            tokio::task::spawn_local(async move { #(#stmts)* })
        })
    } else {
        Some(parse_quote! {
            tokio::spawn(async move { #(#stmts)* })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn parse_macro(code: &str) -> syn::Macro {
        let expr: syn::ExprMacro = syn::parse_str(code).expect("valid macro expr");
        expr.mac
    }

    #[test]
    fn test_recognizes_spawn_block_macro() {
        let mac = parse_macro("__kobo_spawn_block!({ println!(\"hi\") })");
        assert!(is_spawn_block_macro(&mac));
    }

    #[test]
    fn test_ignores_other_macros() {
        let mac = parse_macro("println!(\"hi\")");
        assert!(!is_spawn_block_macro(&mac));
    }

    #[test]
    fn test_lower_spawn_generates_tokio_spawn() {
        let mac = parse_macro("__kobo_spawn_block!({ do_work(data) })");
        let result = lower_spawn_macro(&mac).expect("should lower");
        let output = result.to_token_stream().to_string();
        assert!(output.contains("tokio :: spawn"));
        assert!(output.contains("async move"));
        assert!(output.contains("do_work"));
    }

    #[test]
    fn test_lower_non_spawn_returns_none() {
        let mac = parse_macro("println!(\"hi\")");
        assert!(lower_spawn_macro(&mac).is_none());
    }
}
