/// Lower an @strict fn declaration.
///
/// Task 5.4. All @strict fns use Full mode since BUG-6 fix.
use proc_macro2::TokenStream;
use quote::quote;

use kobo_ir::{CaptureSet, StrictFnMode};
use kobo_parser::KoboItemFn;

use super::block::lower_strict_block;
use super::guard::StrictGuardCounter;
use crate::CodegenOptions;

/// Lower an @strict fn declaration.
///
/// All @strict fns (including async) use Full mode. The AsyncDeferred variant
/// is retained for backward compatibility but is never assigned by the transform.
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
            // BUG-6 fix: transform now always assigns Full. This arm is unreachable
            // but kept for exhaustive matching. If reached, treat as Full.
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
