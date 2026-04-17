use std::collections::HashSet;

use syn::visit::{self, Visit};

/// Scans a [`syn::File`] for `#[derive(...Copy...)]` structs and returns their names.
pub(crate) fn scan_derive_copy(file: &syn::File) -> HashSet<String> {
    let mut collector = DeriveCopyCollector::default();
    collector.visit_file(file);
    collector.copy_types
}

/// Merges derive-scanned Copy types with `Kobo.toml` `[copy_types]` overrides.
pub(crate) fn build_copy_registry(
    derived: HashSet<String>,
    config_types: &[String],
) -> HashSet<String> {
    let mut registry = derived;
    for ty in config_types {
        // Extract the short name for matching (e.g. "glam::Vec3" → "Vec3").
        if let Some(short) = ty.rsplit("::").next() {
            registry.insert(short.to_string());
        }
        registry.insert(ty.clone());
    }
    registry
}

#[derive(Default)]
struct DeriveCopyCollector {
    copy_types: HashSet<String>,
}

impl<'ast> Visit<'ast> for DeriveCopyCollector {
    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        if has_derive_copy(&node.attrs) {
            self.copy_types.insert(node.ident.to_string());
        }
        visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        if has_derive_copy(&node.attrs) {
            self.copy_types.insert(node.ident.to_string());
        }
        visit::visit_item_enum(self, node);
    }
}

fn has_derive_copy(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }
        let Ok(nested) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
        ) else {
            return false;
        };
        nested.iter().any(|path| path.is_ident("Copy"))
    })
}
