use std::collections::HashMap;

use kobo_ir::OwnershipTier;

#[derive(Default)]
pub(crate) struct ScopeStack {
    frames: Vec<ScopeFrame>,
}

#[derive(Clone, Default)]
struct ScopeFrame {
    bindings: HashMap<String, OwnershipTier>,
    /// Binding name → declared type name (e.g. `"Vec"`, `"HashMap"`).
    /// Used for qualified method mutability lookups (BUG 3/27).
    type_names: HashMap<String, String>,
}

impl ScopeStack {
    pub(crate) fn new() -> Self {
        Self { frames: Vec::new() }
    }

    pub(crate) fn push(&mut self) {
        self.frames.push(ScopeFrame::default());
    }

    pub(crate) fn pop(&mut self) {
        self.frames.pop();
    }

    pub(crate) fn insert(&mut self, ident: &syn::Ident, tier: OwnershipTier) {
        if let Some(frame) = self.frames.last_mut() {
            frame.bindings.insert(ident.to_string(), tier);
        }
    }

    /// Record the declared type name for a binding (e.g. `"Vec"` for `let v: Vec<String>`).
    pub(crate) fn insert_type_name(&mut self, ident: &syn::Ident, type_name: String) {
        if let Some(frame) = self.frames.last_mut() {
            frame.type_names.insert(ident.to_string(), type_name);
        }
    }

    pub(crate) fn lookup(&self, ident: &syn::Ident) -> Option<OwnershipTier> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.bindings.get(&ident.to_string()).copied())
    }

    /// Look up the declared type name for a binding.
    pub(crate) fn lookup_type_name(&self, ident: &syn::Ident) -> Option<&str> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.type_names.get(&ident.to_string()).map(|s| s.as_str()))
    }
}

/// Extract the outermost type name from a `syn::Type`.
///
/// Returns `Some("Vec")` for `Vec<String>`, `Some("HashMap")` for `&HashMap<K,V>`, etc.
pub(crate) fn type_name_from_syn(ty: &syn::Type) -> Option<String> {
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
