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

/// Infer the outer type name from common initializer expression shapes.
pub(crate) fn type_name_from_expr(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Call(call) => type_name_from_callable(call.func.as_ref()),
        syn::Expr::Struct(expr_struct) => expr_struct
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        syn::Expr::Macro(expr_macro) => expr_macro
            .mac
            .path
            .is_ident("vec")
            .then(|| "Vec".to_owned()),
        syn::Expr::Reference(reference) => type_name_from_expr(reference.expr.as_ref()),
        syn::Expr::Paren(paren) => type_name_from_expr(paren.expr.as_ref()),
        syn::Expr::Group(group) => type_name_from_expr(group.expr.as_ref()),
        _ => None,
    }
}

fn type_name_from_callable(func: &syn::Expr) -> Option<String> {
    match func {
        syn::Expr::Path(path) => type_name_from_constructor_path(&path.path),
        syn::Expr::Paren(paren) => type_name_from_callable(paren.expr.as_ref()),
        syn::Expr::Group(group) => type_name_from_callable(group.expr.as_ref()),
        _ => None,
    }
}

fn type_name_from_constructor_path(path: &syn::Path) -> Option<String> {
    if path.segments.len() >= 2 {
        return path
            .segments
            .iter()
            .nth(path.segments.len() - 2)
            .map(|segment| segment.ident.to_string());
    }

    let segment = path.segments.last()?;
    matches!(segment.arguments, syn::PathArguments::AngleBracketed(_))
        .then(|| segment.ident.to_string())
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::{type_name_from_expr, type_name_from_syn};

    #[test]
    fn type_name_from_syn_extracts_outer_type() {
        let ty: syn::Type = parse_quote!(&std::collections::HashMap<String, usize>);
        assert_eq!(type_name_from_syn(&ty).as_deref(), Some("HashMap"));
    }

    #[test]
    fn type_name_from_expr_handles_associated_constructor_calls() {
        let expr: syn::Expr = parse_quote!(Reader::new());
        assert_eq!(type_name_from_expr(&expr).as_deref(), Some("Reader"));
    }

    #[test]
    fn type_name_from_expr_handles_vec_macro() {
        let expr: syn::Expr = parse_quote!(vec![1, 2, 3]);
        assert_eq!(type_name_from_expr(&expr).as_deref(), Some("Vec"));
    }
}
