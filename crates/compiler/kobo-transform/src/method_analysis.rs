use std::collections::HashMap;
use std::fmt;

/// A type-qualified method path like `Vec::push` or `HashMap::insert`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct QualifiedPath {
    pub type_name: String,
    pub method_name: String,
}

impl QualifiedPath {
    pub fn new(type_name: impl Into<String>, method_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            method_name: method_name.into(),
        }
    }
}

impl fmt::Display for QualifiedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.type_name, self.method_name)
    }
}

/// Extract the type name from an `impl` block's self type.
///
/// Returns `Some("Foo")` for `impl Foo { ... }` or `impl<T> Foo<T> { ... }`.
/// Returns `None` for trait impls with complex paths.
pub fn extract_type_name(item_impl: &syn::ItemImpl) -> Option<String> {
    // Trait impls (e.g. `impl Trait for Type`) — extract from self_ty
    let ty = &*item_impl.self_ty;
    type_name_from_syn(ty)
}

fn type_name_from_syn(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => {
            let segment = path.path.segments.last()?;
            Some(segment.ident.to_string())
        }
        syn::Type::Reference(r) => type_name_from_syn(&r.elem),
        syn::Type::Paren(p) => type_name_from_syn(&p.elem),
        _ => None,
    }
}

/// Maps method names to their receiver mutability, built from `impl` blocks.
pub(crate) struct MethodRegistry {
    /// Qualified `"Type::method"` → is_mut_self.
    /// When the same method name appears in multiple impls with different
    /// receivers, the conservative answer (`true`) wins.
    qualified: HashMap<String, bool>,
    /// Bare method name → is_mut_self (fallback for unknown receiver type).
    bare: HashMap<String, bool>,
}

impl MethodRegistry {
    /// Scan all `impl` blocks in a file and record receiver mutability.
    pub fn from_impl_blocks(file: &syn::File) -> Self {
        let mut qualified = HashMap::new();
        let mut bare = HashMap::new();
        for item in &file.items {
            if let syn::Item::Impl(item_impl) = item {
                let type_name = extract_type_name(item_impl);
                for impl_item in &item_impl.items {
                    if let syn::ImplItem::Fn(method) = impl_item {
                        let name = method.sig.ident.to_string();
                        let is_mut = has_mut_self_receiver(&method.sig);

                        // Store qualified key if type name is known
                        if let Some(ref ty_name) = type_name {
                            let qkey = format!("{ty_name}::{name}");
                            let entry = qualified.entry(qkey).or_insert(false);
                            if is_mut {
                                *entry = true;
                            }
                        }

                        // Always store bare key as fallback
                        let entry = bare.entry(name).or_insert(false);
                        if is_mut {
                            *entry = true;
                        }
                    }
                }
            }
        }
        Self { qualified, bare }
    }

    /// Merge config overrides into the registry. Each entry is a method name
    /// (e.g. `"insert"`) or qualified `"HashMap::insert"`. Qualified entries
    /// are stored in the qualified map.
    pub fn with_config_overrides(mut self, config: &[String]) -> Self {
        for entry in config {
            if let Some((ty, method)) = entry.rsplit_once("::") {
                self.qualified.insert(entry.clone(), true);
                // Also set bare as conservative fallback
                self.bare.insert(method.to_string(), true);
                let _ = ty; // suppress unused
            } else {
                self.bare.insert(entry.clone(), true);
            }
        }
        self
    }

    /// Look up whether a method is mutating using a qualified key first, then bare.
    /// Returns `None` if unknown in both maps.
    pub fn is_mutating_qualified(&self, type_name: &str, method_name: &str) -> Option<bool> {
        let qkey = format!("{type_name}::{method_name}");
        self.qualified
            .get(&qkey)
            .copied()
            .or_else(|| self.bare.get(method_name).copied())
    }

    /// Look up whether a method is mutating by bare name. Returns `None` if unknown.
    pub fn is_mutating_by_name(&self, method_name: &str) -> Option<bool> {
        self.bare.get(method_name).copied()
    }

    /// Consume the registry and return the raw bare map for storage in KIR.
    pub fn into_map(self) -> HashMap<String, bool> {
        self.bare
    }
}

/// Return `true` if the function signature has `&mut self` or `mut self`.
fn has_mut_self_receiver(sig: &syn::Signature) -> bool {
    for input in &sig.inputs {
        if let syn::FnArg::Receiver(recv) = input {
            // `&mut self`
            if recv.reference.is_some() && recv.mutability.is_some() {
                return true;
            }
            // `mut self` (owned mutable receiver)
            if recv.reference.is_none() && recv.mutability.is_some() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{extract_type_name, MethodRegistry, QualifiedPath};

    #[test]
    fn qualified_path_display() {
        let qp = QualifiedPath::new("Vec", "push");
        assert_eq!(qp.to_string(), "Vec::push");
    }

    #[test]
    fn extract_type_name_simple() {
        let file: syn::File = syn::parse_str("struct Foo; impl Foo { fn bar(&self) {} }")
            .expect("test source should parse");
        if let syn::Item::Impl(item_impl) = &file.items[1] {
            assert_eq!(extract_type_name(item_impl), Some("Foo".to_string()));
        } else {
            panic!("expected impl item");
        }
    }

    #[test]
    fn extract_type_name_generic() {
        let file: syn::File =
            syn::parse_str("struct Bar<T>(T); impl<T> Bar<T> { fn baz(&self) {} }")
                .expect("test source should parse");
        if let syn::Item::Impl(item_impl) = &file.items[1] {
            assert_eq!(extract_type_name(item_impl), Some("Bar".to_string()));
        } else {
            panic!("expected impl item");
        }
    }

    #[test]
    fn owned_mut_self_receiver_is_recorded_as_mutating() {
        let file: syn::File = syn::parse_str(
            r#"
struct Counter;
impl Counter {
    fn advance(mut self) -> Self { self }
}
"#,
        )
        .expect("test source should parse");

        let map = MethodRegistry::from_impl_blocks(&file).into_map();

        assert_eq!(map.get("advance"), Some(&true));
    }

    #[test]
    fn registry_keeps_type_qualified_entries_separate() {
        let file: syn::File = syn::parse_str(
            r#"
struct Reader;
impl Reader {
    fn touch(&self) {}
}

struct Writer;
impl Writer {
    fn touch(&mut self) {}
}
"#,
        )
        .expect("test source should parse");

        let registry = MethodRegistry::from_impl_blocks(&file);

        // Qualified lookups should be distinct
        assert_eq!(registry.is_mutating_qualified("Reader", "touch"), Some(false));
        assert_eq!(registry.is_mutating_qualified("Writer", "touch"), Some(true));

        // Bare lookup returns conservative (true wins)
        assert_eq!(registry.is_mutating_by_name("touch"), Some(true));
    }

    #[test]
    fn qualified_lookup_does_not_collide() {
        let file: syn::File = syn::parse_str(
            r#"
struct Vec;
impl Vec {
    fn push(&mut self) {}
}

struct HashMap;
impl HashMap {
    fn push(&self) {}
}
"#,
        )
        .expect("test source should parse");

        let registry = MethodRegistry::from_impl_blocks(&file);
        assert_eq!(registry.is_mutating_qualified("Vec", "push"), Some(true));
        assert_eq!(registry.is_mutating_qualified("HashMap", "push"), Some(false));
    }
}
