/// Control-flow enum helpers for @strict blocks with break/continue.
///
/// Encapsulated so the lowering strategy can evolve without changing callers.
use proc_macro2::TokenStream;
use quote::quote;

/// Emit the `__KoboStrictCf<V>` enum definition.
///
/// Emitted once per function when any @strict block uses break or continue.
/// Placed at the START of the function body (not per-block) per Rebuttal 2-3.
#[cfg_attr(not(test), allow(dead_code))]
pub fn emit_cf_enum_def() -> TokenStream {
    quote! {
        #[allow(non_camel_case_types, dead_code)]
        enum __KoboStrictCf<V> {
            Continue,
            Break,
            Fallthrough(V),
        }
    }
}

/// Emit the match dispatch after guard drops for break/continue blocks.
pub fn emit_cf_dispatch(result_ident: &proc_macro2::Ident) -> TokenStream {
    quote! {
        match #result_ident {
            __KoboStrictCf::Break => break,
            __KoboStrictCf::Continue => continue,
            __KoboStrictCf::Fallthrough(v) => v,
        }
    }
}
