use super::*;

impl BindingEnv {
    pub(super) fn with_imports(imports: ImportMap) -> Self {
        Self {
            imports,
            ..Self::default()
        }
    }

    pub(super) fn bind(&mut self, local: String, obligation_key: String) {
        self.clear_local(&local);
        self.bindings.insert(local, obligation_key);
    }

    pub(super) fn bind_obligation(
        &mut self,
        local: String,
        obligation_key: String,
        terminal_actions: Vec<String>,
    ) {
        self.clear_local(&local);
        self.bindings.insert(local.clone(), obligation_key.clone());
        self.terminal_actions
            .insert(obligation_key, terminal_actions);
    }

    pub(super) fn bind_bool(&mut self, local: String, value: bool) {
        self.clear_local(&local);
        self.bools.insert(local, value);
    }

    pub(super) fn bind_external(&mut self, local: String, value: ExternalBoundaryValue) {
        self.clear_local(&local);
        self.external_values.insert(local, value);
    }

    pub(super) fn bind_type(&mut self, local: String, type_name: String) {
        self.clear_local(&local);
        self.local_types.insert(local, type_name);
    }

    pub(super) fn clear_local(&mut self, local: &str) {
        self.bindings.remove(local);
        self.bools.remove(local);
        self.external_values.remove(local);
        self.local_types.remove(local);
    }

    pub(super) fn resolve(&self, local: &str) -> Option<String> {
        self.bindings.get(local).cloned()
    }

    pub(super) fn terminal_actions(&self, obligation_key: &str) -> Option<Vec<String>> {
        self.terminal_actions.get(obligation_key).cloned()
    }

    pub(super) fn active_obligations(&self) -> HashSet<String> {
        self.terminal_actions.keys().cloned().collect()
    }

    pub(super) fn has_active_obligation(&self, obligation_key: &str) -> bool {
        self.terminal_actions.contains_key(obligation_key)
    }

    pub(super) fn discharge_obligation(&mut self, obligation_key: &str) {
        self.terminal_actions.remove(obligation_key);
    }

    pub(super) fn resolve_bool(&self, local: &str) -> Option<bool> {
        self.bools.get(local).copied()
    }

    pub(super) fn resolve_external(&self, local: &str) -> Option<&ExternalBoundaryValue> {
        self.external_values.get(local)
    }

    pub(super) fn resolve_type(&self, local: &str) -> Option<&str> {
        self.local_types.get(local).map(String::as_str)
    }

    pub(super) fn bind_imports_from_use(&mut self, item_use: &ItemUse) {
        collect_use_tree_aliases(&item_use.tree, Vec::new(), &mut self.imports);
    }
}
