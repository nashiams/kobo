use super::{
    must_call_actions, path_ends_with, path_last_ident, File, HashMap, Item, ItemFn, MethodShape,
    MethodShapeMap, MustCallObligation, Punctuated,
};
pub(super) fn collect_functions(file: &File) -> HashMap<String, &ItemFn> {
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) => Some((function.sig.ident.to_string(), function)),
            _ => None,
        })
        .collect()
}

pub(super) fn collect_method_shapes(file: &File) -> MethodShapeMap {
    let mut methods_by_type = MethodShapeMap::new();
    for item in &file.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(owner_type) = impl_owner_type(item_impl.self_ty.as_ref()) else {
            continue;
        };
        let method_shapes = methods_by_type.entry(owner_type).or_default();
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            method_shapes.insert(
                method.sig.ident.to_string(),
                MethodShape {
                    return_type: return_type_name(&method.sig.output),
                    consumes_receiver: method_consumes_receiver(&method.sig.inputs),
                },
            );
        }
    }
    methods_by_type
}

pub(super) fn impl_owner_type(self_ty: &syn::Type) -> Option<String> {
    match self_ty {
        syn::Type::Path(path) => path_last_ident(&path.path),
        _ => None,
    }
}

pub(super) fn return_type_name(output: &syn::ReturnType) -> Option<String> {
    match output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ty) => match ty.as_ref() {
            syn::Type::Path(path) => path_last_ident(&path.path),
            _ => None,
        },
    }
}

pub(super) fn method_consumes_receiver(inputs: &Punctuated<syn::FnArg, syn::token::Comma>) -> bool {
    inputs.first().is_some_and(|input| {
        matches!(
            input,
            syn::FnArg::Receiver(receiver) if receiver.reference.is_none()
        )
    })
}

pub(super) fn must_call_type_map(
    file: &File,
    obligations: &[MustCallObligation],
) -> HashMap<String, Vec<String>> {
    if !obligations.is_empty() {
        return obligations
            .iter()
            .map(|obligation| {
                (
                    obligation.owner_type.clone(),
                    obligation
                        .actions
                        .iter()
                        .map(|action| action.name.clone())
                        .collect(),
                )
            })
            .collect();
    }

    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(item_struct) => item_struct
                .attrs
                .iter()
                .find(|attr| path_ends_with(attr.path(), &["kobo", "must_call"]))
                .and_then(must_call_actions)
                .map(|actions| (item_struct.ident.to_string(), actions)),
            _ => None,
        })
        .collect()
}
