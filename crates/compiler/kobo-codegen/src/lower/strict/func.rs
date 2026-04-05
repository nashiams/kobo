/// Lower an @strict fn declaration.
///
/// Task 5.4. Two modes (R-14): Full and AsyncDeferred.
use proc_macro2::TokenStream;
use quote::quote;

use kobo_ir::{CaptureSet, StrictFnMode};
use kobo_parser::KoboItemFn;

use super::block::lower_strict_block;
use super::guard::StrictGuardCounter;
use crate::CodegenOptions;

/// Lower an @strict fn declaration.
///
/// Two modes (R-14):
/// - **AsyncDeferred**: strip the `#[__kobo_strict]` marker from the
///   attributes and emit the function unchanged.
/// - **Full**: guard-extract all bindings in `capture_set` around the fn
///   body stmts.
pub fn lower_strict_fn(
    func: &KoboItemFn,
    mode: StrictFnMode,
    capture_set: Option<&CaptureSet>,
    counter: &mut StrictGuardCounter,
    _options: &CodegenOptions,
) -> TokenStream {
    let mut inner = func.inner.clone();

    // Strip the @strict marker attribute from the fn (Contract C06).
    inner.attrs.retain(|a| {
        !a.path().is_ident("__kobo_strict")
            && !(a.path().segments.len() == 1
                && a.path().segments[0].ident == "__kobo_strict")
    });

    match mode {
        StrictFnMode::AsyncDeferred => {
            eprintln!(
                "warning: @strict on async fn `{}` is deferred to v0.7; \
                 marker stripped, function compiled without @strict optimization",
                func.inner.sig.ident
            );
            quote! { #inner }
        }
        StrictFnMode::Full => {
            if let Some(cs) = capture_set {
                if !cs.bindings.is_empty() {
                    let wrapped_stmts_ts =
                        lower_strict_block(&inner.block.stmts, cs, counter, _options);
                    if let Ok(wrapped_block) = syn::parse2::<syn::Block>(wrapped_stmts_ts.clone()) {
                        inner.block = Box::new(wrapped_block);
                    } else {
                        let body_stmts = &inner.block.stmts;
                        inner.block = syn::parse_quote! { {
                            #wrapped_stmts_ts
                            let _ = { #(#body_stmts)* };
                        } };
                    }
                }
            }
            quote! { #inner }
        }
    }
}
