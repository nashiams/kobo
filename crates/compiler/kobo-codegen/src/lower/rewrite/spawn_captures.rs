use quote::ToTokens;

use super::clone_inject;
use super::ScopeStack;

/// Collect bindings captured by a spawn block's token stream.
///
/// Scans tokens for identifiers and checks each against the scope stack.
/// Bindings found in scope are returned as `CapturedBinding` for clone analysis.
/// Conservative: assumes all in-scope bindings referenced in the body are used after spawn.
pub(super) fn collect_spawn_captures(
    tokens: &proc_macro2::TokenStream,
    scopes: &ScopeStack,
) -> Vec<clone_inject::CapturedBinding> {
    use kobo_ir::OwnershipTier;
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut captures = Vec::new();
    collect_idents_from_tokens(tokens, &mut seen);

    for name in seen {
        let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
        if let Some(tier) = scopes.lookup(&ident) {
            let is_copy = matches!(tier, OwnershipTier::PlainOwned);
            captures.push(clone_inject::CapturedBinding {
                name,
                tier,
                is_copy,
                used_after_spawn: true, // conservative assumption
            });
        }
    }
    captures
}

pub(super) fn collect_block_captures(
    block: &syn::Block,
    scopes: &ScopeStack,
) -> Vec<clone_inject::CapturedBinding> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    collect_idents_from_tokens(&block.to_token_stream(), &mut seen);
    seen.into_iter()
        .filter_map(|name| {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            let tier = scopes.lookup(&ident)?;
            let is_copy = matches!(tier, kobo_ir::OwnershipTier::PlainOwned);
            Some(clone_inject::CapturedBinding {
                name,
                tier,
                is_copy,
                used_after_spawn: true,
            })
        })
        .collect()
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn captured_bindings_need_spawn_local(
    captured: &[clone_inject::CapturedBinding],
) -> bool {
    captured.iter().any(|binding| {
        matches!(
            binding.tier,
            kobo_ir::OwnershipTier::RcShared | kobo_ir::OwnershipTier::RcMutShared
        )
    })
}

/// Recursively collect all identifiers from a token stream.
fn collect_idents_from_tokens(
    tokens: &proc_macro2::TokenStream,
    out: &mut std::collections::HashSet<String>,
) {
    for token in tokens.clone() {
        match token {
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                // Skip Rust keywords.
                if !is_rust_keyword(&name) {
                    out.insert(name);
                }
            }
            proc_macro2::TokenTree::Group(group) => {
                collect_idents_from_tokens(&group.stream(), out);
            }
            _ => {}
        }
    }
}

pub(super) fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
    )
}
