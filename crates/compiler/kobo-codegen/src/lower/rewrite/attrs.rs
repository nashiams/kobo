use crate::executor::{executor_attribute, ExecutorChoice};
use syn::parse_quote;

pub(super) fn executor_main_attr(choice: ExecutorChoice, is_main: bool) -> Option<syn::Attribute> {
    match executor_attribute(choice, is_main) {
        Some("#[tokio::main]") => Some(parse_quote!(#[tokio::main])),
        Some("#[async_std::main]") => Some(parse_quote!(#[async_std::main])),
        Some(other) => panic!("unsupported executor attribute: {other}"),
        None => None,
    }
}

pub(super) fn is_executor_main_attr(attr: &syn::Attribute) -> bool {
    let segments: Vec<_> = attr
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    matches!(segments.as_slice(), [executor, main] if main == "main" && (executor == "tokio" || executor == "async_std"))
}
