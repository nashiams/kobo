use kobo_ir::OwnershipHint;

pub(crate) fn parse_hint(attrs: &[syn::Attribute]) -> Option<(OwnershipHint, proc_macro2::Span)> {
    attrs.iter().find_map(parse_hint_attr)
}

fn parse_hint_attr(attr: &syn::Attribute) -> Option<(OwnershipHint, proc_macro2::Span)> {
    if !is_hint_attr(attr) {
        return None;
    }

    let mut parsed_hint = None;
    let _ = attr.parse_nested_meta(|meta| {
        if !meta.path.is_ident("ownership") {
            return Ok(());
        }
        let value = meta.value()?;
        let hint_literal: syn::LitStr = value.parse()?;
        parsed_hint = ownership_hint_from_str(hint_literal.value().as_str())
            .map(|hint| (hint, hint_literal.span()));
        Ok(())
    });

    parsed_hint
}

fn is_hint_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .first()
        .is_some_and(|segment| segment.ident == "kobo")
        && attr
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "hint")
}

fn ownership_hint_from_str(value: &str) -> Option<OwnershipHint> {
    match value {
        "move" => Some(OwnershipHint::Move),
        "exclusive" => Some(OwnershipHint::Exclusive),
        "shared" => Some(OwnershipHint::Shared),
        "async" => Some(OwnershipHint::Async),
        _ => None,
    }
}
