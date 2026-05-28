use super::spawn_captures::is_rust_keyword;

pub(super) fn ident_from_kobo(name: &str) -> Option<syn::Ident> {
    if is_rust_keyword(name) {
        return None;
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
        return None;
    }
    Some(syn::Ident::new(name, proc_macro2::Span::call_site()))
}
