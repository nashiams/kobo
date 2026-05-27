use std::path::Path;

pub(super) fn read_project_config(source_path: &Path) -> Option<toml::Value> {
    for directory in source_path.parent().into_iter().flat_map(Path::ancestors) {
        let config_path = directory.join("Kobo.toml");
        let Ok(source) = std::fs::read_to_string(&config_path) else {
            continue;
        };
        if let Ok(config) = source.parse::<toml::Value>() {
            return Some(config);
        }
    }
    None
}

pub(super) fn config_string(config: &Option<toml::Value>, path: &[&str]) -> Option<String> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_owned)
}

pub(super) fn config_bool(config: &Option<toml::Value>, path: &[&str]) -> Option<bool> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
}

pub(super) fn file_uses_nightly_features(file: &syn::File) -> bool {
    file.attrs
        .iter()
        .any(|attr| syn_path_ends_with(attr.path(), &["feature"]))
}

pub(super) fn file_hidden_heap_site_count(file: &syn::File) -> usize {
    struct HiddenHeapVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for HiddenHeapVisitor {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(function) = call.func.as_ref() {
                if path_is_heap_constructor(&function.path) {
                    self.count += 1;
                }
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if matches!(
                call.method.to_string().as_str(),
                "with_capacity" | "try_with_capacity" | "collect" | "to_vec" | "to_string"
            ) {
                self.count += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if path_last_ident_is(&mac.path, "vec") || path_last_ident_is(&mac.path, "format") {
                self.count += 1;
            }
            syn::visit::visit_macro(self, mac);
        }
    }

    let mut visitor = HiddenHeapVisitor { count: 0 };
    syn::visit::visit_file(&mut visitor, file);
    visitor.count
}

pub(super) fn file_cast_site_count(file: &syn::File) -> usize {
    struct CastVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for CastVisitor {
        fn visit_expr_cast(&mut self, cast: &'ast syn::ExprCast) {
            self.count += 1;
            syn::visit::visit_expr_cast(self, cast);
        }
    }

    let mut visitor = CastVisitor { count: 0 };
    syn::visit::visit_file(&mut visitor, file);
    visitor.count
}

pub(super) fn file_has_hidden_impls(file: &syn::File) -> bool {
    file.items.iter().any(item_has_hidden_impl)
}

pub(super) fn item_has_hidden_impl(item: &syn::Item) -> bool {
    match item {
        syn::Item::Impl(item) => {
            item.attrs
                .iter()
                .any(|attr| syn_path_ends_with(attr.path(), &["hidden_impl"]))
                || item.trait_.as_ref().is_some_and(|(_, trait_path, _)| {
                    path_has_segment(trait_path, "external")
                        || path_has_segment(trait_path, "ExternalTrait")
                })
                || type_has_segment(item.self_ty.as_ref(), "external")
                || type_has_segment(item.self_ty.as_ref(), "ExternalType")
        }
        syn::Item::Mod(item) => item
            .content
            .as_ref()
            .is_some_and(|(_, items)| items.iter().any(item_has_hidden_impl)),
        _ => false,
    }
}

pub(super) fn file_has_hidden_globals(file: &syn::File) -> bool {
    struct HiddenGlobalVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for HiddenGlobalVisitor {
        fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
            if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
                self.found = true;
            }
            syn::visit::visit_item_static(self, item);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if path_last_ident_is(&mac.path, "lazy_static")
                || path_last_ident_is(&mac.path, "thread_local")
            {
                self.found = true;
            }
            syn::visit::visit_macro(self, mac);
        }
    }

    let mut visitor = HiddenGlobalVisitor { found: false };
    syn::visit::visit_file(&mut visitor, file);
    visitor.found
}

pub(super) fn file_has_context_binding(file: &syn::File) -> bool {
    struct ContextBindingVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for ContextBindingVisitor {
        fn visit_pat_ident(&mut self, pat: &'ast syn::PatIdent) {
            if pat.ident == "ctx" {
                self.found = true;
            }
            syn::visit::visit_pat_ident(self, pat);
        }
    }

    let mut visitor = ContextBindingVisitor { found: false };
    syn::visit::visit_file(&mut visitor, file);
    visitor.found
}

pub(super) fn file_has_attr(file: &syn::File, suffix: &[&str]) -> bool {
    !file_attrs_with_path(file, suffix).is_empty()
}

pub(super) fn file_attrs_with_path<'a>(
    file: &'a syn::File,
    suffix: &[&str],
) -> Vec<&'a syn::Attribute> {
    let mut attrs = Vec::new();
    attrs.extend(
        file.attrs
            .iter()
            .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
    );
    for item in &file.items {
        collect_item_attrs_with_path(item, suffix, &mut attrs);
    }
    attrs
}

pub(super) fn collect_item_attrs_with_path<'a>(
    item: &'a syn::Item,
    suffix: &[&str],
    attrs: &mut Vec<&'a syn::Attribute>,
) {
    attrs.extend(
        item_attributes(item)
            .iter()
            .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
    );
    match item {
        syn::Item::Impl(item) => {
            for impl_item in &item.items {
                attrs.extend(
                    impl_item_attributes(impl_item)
                        .iter()
                        .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
                );
            }
        }
        syn::Item::Mod(item) => {
            if let Some((_, items)) = &item.content {
                for item in items {
                    collect_item_attrs_with_path(item, suffix, attrs);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        _ => &[],
    }
}

pub(super) fn impl_item_attributes(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        _ => &[],
    }
}

pub(super) fn attr_path_arguments(attr: &syn::Attribute) -> Vec<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return Vec::new();
    };
    list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        .map(|entries| {
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    syn::Meta::Path(path) => path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn attr_has_path_argument(attr: &syn::Attribute, argument: &str) -> bool {
    attr_path_arguments(attr)
        .iter()
        .any(|candidate| candidate == argument)
}

pub(super) fn attr_string_field(attr: &syn::Attribute, key: &str) -> Option<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    for entry in entries {
        let field = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
        if field != key {
            continue;
        }
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        {
            return Some(value.value());
        }
    }
    None
}

pub(super) fn path_is_heap_constructor(path: &syn::Path) -> bool {
    let last = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string());
    let Some(last) = last else {
        return false;
    };
    matches!(
        last.as_str(),
        "new" | "from" | "with_capacity" | "try_with_capacity"
    ) && (path_has_segment(path, "Vec")
        || path_has_segment(path, "Box")
        || path_has_segment(path, "String")
        || path_has_segment(path, "alloc"))
}

pub(super) fn path_last_ident_is(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == expected)
}

pub(super) fn path_has_segment(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .iter()
        .any(|segment| segment.ident == expected)
}

pub(super) fn type_has_segment(ty: &syn::Type, expected: &str) -> bool {
    match ty {
        syn::Type::Path(ty) => path_has_segment(&ty.path, expected),
        syn::Type::Reference(ty) => type_has_segment(&ty.elem, expected),
        syn::Type::Group(ty) => type_has_segment(&ty.elem, expected),
        syn::Type::Paren(ty) => type_has_segment(&ty.elem, expected),
        _ => false,
    }
}

pub(in crate::proof) fn syn_path_ends_with(path: &syn::Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    segments.len() >= suffix.len()
        && segments[segments.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(left, right)| left == right)
}
