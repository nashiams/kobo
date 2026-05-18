use quote::ToTokens;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParallelLowering {
    None,
    SerialPolicy,
    Parallel,
    BoundaryPolicy,
}

pub(crate) fn lower_for_loop(for_loop: &mut syn::ExprForLoop) -> ParallelLowering {
    let Some(attr) = parallel_attr(&for_loop.attrs).cloned() else {
        return ParallelLowering::None;
    };
    for_loop.attrs.retain(|candidate| !is_parallel_attr(candidate));

    if attr_has_value(&attr, "order", "serial") {
        return ParallelLowering::SerialPolicy;
    }
    if attr_has_key(&attr, "policy") {
        return ParallelLowering::BoundaryPolicy;
    }
    if lower_iterator_expr_to_rayon(&mut for_loop.expr) {
        ParallelLowering::Parallel
    } else {
        ParallelLowering::SerialPolicy
    }
}

pub(crate) fn mark_serial_policy(for_loop: &mut syn::ExprForLoop) {
    for_loop
        .body
        .stmts
        .insert(0, syn::parse_quote!(let _ = "parallel-order=serial";));
}

pub(crate) fn mark_boundary_policy(for_loop: &mut syn::ExprForLoop, policy: &str) {
    let marker = format!("parallel-policy={policy}");
    for_loop
        .body
        .stmts
        .insert(0, syn::parse_quote!(let _ = #marker;));
}

pub(crate) fn policy_value(attrs: &[syn::Attribute]) -> Option<String> {
    attrs
        .iter()
        .find(|attr| is_parallel_attr(attr))
        .and_then(|attr| attr_value(attr, "policy"))
}

pub(crate) fn insert_rayon_import(file: &mut syn::File) {
    if file.items.iter().any(is_rayon_prelude_use) {
        return;
    }
    file.items.insert(0, syn::parse_quote!(use rayon::prelude::*;));
}

fn parallel_attr(attrs: &[syn::Attribute]) -> Option<&syn::Attribute> {
    attrs.iter().find(|attr| is_parallel_attr(attr))
}

fn is_parallel_attr(attr: &syn::Attribute) -> bool {
    let segments = attr.path().segments.iter().collect::<Vec<_>>();
    segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "parallel"
}

fn attr_has_key(attr: &syn::Attribute, key: &str) -> bool {
    attr_value(attr, key).is_some()
}

fn attr_has_value(attr: &syn::Attribute, key: &str, expected: &str) -> bool {
    attr_value(attr, key).as_deref() == Some(expected)
}

fn attr_value(attr: &syn::Attribute, key: &str) -> Option<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    for entry in entries {
        if entry.path.get_ident().is_none_or(|ident| ident != key) {
            continue;
        }
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        return Some(value.value());
    }
    None
}

fn lower_iterator_expr_to_rayon(expr: &mut Box<syn::Expr>) -> bool {
    let rendered = expr.to_token_stream().to_string();
    if !(rendered.contains(". iter (") || rendered.contains(".iter(")) {
        return false;
    }
    let lowered = rendered
        .replace(". iter (", ". par_iter (")
        .replace(".iter(", ".par_iter(");
    let Ok(parsed) = syn::parse_str::<syn::Expr>(&lowered) else {
        return false;
    };
    *expr = Box::new(parsed);
    true
}

fn is_rayon_prelude_use(item: &syn::Item) -> bool {
    let syn::Item::Use(item_use) = item else {
        return false;
    };
    item_use.to_token_stream().to_string().contains("rayon :: prelude")
}
