use std::collections::HashMap;

/// Maps method names to their receiver mutability, built from `impl` blocks.
pub(crate) struct MethodRegistry {
    /// method_name → is_mut_self.
    /// When the same method name appears in multiple impls with different
    /// receivers, the conservative answer (`true`) wins.
    methods: HashMap<String, bool>,
}

impl MethodRegistry {
    /// Scan all `impl` blocks in a file and record receiver mutability.
    pub fn from_impl_blocks(file: &syn::File) -> Self {
        let mut methods = HashMap::new();
        for item in &file.items {
            if let syn::Item::Impl(item_impl) = item {
                for impl_item in &item_impl.items {
                    if let syn::ImplItem::Fn(method) = impl_item {
                        let name = method.sig.ident.to_string();
                        let is_mut = has_mut_self_receiver(&method.sig);
                        // Conservative merge: if any impl says &mut self, keep true.
                        let entry = methods.entry(name).or_insert(false);
                        if is_mut {
                            *entry = true;
                        }
                    }
                }
            }
        }
        Self { methods }
    }

    /// Merge config overrides into the registry. Each entry is a method name
    /// (e.g. `"insert"`) or qualified `"HashMap::insert"`. Qualified entries
    /// are stored by their unqualified method name.
    pub fn with_config_overrides(mut self, config: &[String]) -> Self {
        for entry in config {
            let method_name = entry
                .rsplit_once("::")
                .map(|(_, name)| name)
                .unwrap_or(entry);
            self.methods.insert(method_name.to_string(), true);
        }
        self
    }

    /// Look up whether a method is mutating. Returns `None` if unknown.
    pub fn is_mutating_by_name(&self, method_name: &str) -> Option<bool> {
        self.methods.get(method_name).copied()
    }

    /// Consume the registry and return the raw map for storage in KIR.
    pub fn into_map(self) -> HashMap<String, bool> {
        self.methods
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
