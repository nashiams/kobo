use std::collections::HashMap;

use kobo_ir::OwnershipTier;

#[derive(Default)]
pub(crate) struct ScopeStack {
    frames: Vec<ScopeFrame>,
}

#[derive(Clone, Default)]
struct ScopeFrame {
    bindings: HashMap<String, OwnershipTier>,
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

    pub(crate) fn lookup(&self, ident: &syn::Ident) -> Option<OwnershipTier> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.bindings.get(&ident.to_string()).copied())
    }
}
