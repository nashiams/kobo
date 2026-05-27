//! S-20: Generate split-borrow destructuring for impl methods.
//!
//! Inserts `let Self { field_a, field_b,.. } = self;` and replaces
//! all `self.field` references with bare field identifiers.

use kobo_analysis::split_borrow::{FieldAccessKind, SplitBorrowSite};
use std::collections::HashMap;
use syn::visit_mut::VisitMut;

/// Apply split-borrow destructuring to a method based on detected sites.
///
/// Returns `true` if rewriting was applied.
pub(crate) fn generate_split_borrow(method: &mut syn::ImplItemFn, site: &SplitBorrowSite) -> bool {
    // Only &mut self methods get split-borrow.
    if !has_mut_self_receiver(method) {
        return false;
    }

    // Build field → max access kind map (BorrowMut > Write > Read).
    let mut field_kinds: HashMap<String, FieldAccessKind> = HashMap::new();
    for access in &site.fields_accessed {
        let entry = field_kinds
            .entry(access.field_name.clone())
            .or_insert(FieldAccessKind::Read);
        // Escalate: Read < Write < BorrowMut
        if access_rank(access.access_kind) > access_rank(*entry) {
            *entry = access.access_kind;
        }
    }

    if field_kinds.is_empty() {
        return false;
    }

    // Build destructure pattern: `let Self { field_a, ref mut field_b,.. } = self;`
    let destructure = build_destructure_stmt(&field_kinds);

    // Replace all `self.field` with bare `field` in the method body.
    let mut replacer = SelfFieldReplacer {
        field_kinds: field_kinds.clone(),
    };
    replacer.visit_block_mut(&mut method.block);

    // Insert destructure as the first statement.
    method.block.stmts.insert(0, destructure);

    true
}

fn access_rank(kind: FieldAccessKind) -> u8 {
    match kind {
        FieldAccessKind::Read => 0,
        FieldAccessKind::Write => 1,
        FieldAccessKind::BorrowMut => 2,
    }
}

fn has_mut_self_receiver(method: &syn::ImplItemFn) -> bool {
    method
        .sig
        .inputs
        .first()
        .map(|arg| matches!(arg, syn::FnArg::Receiver(r) if r.mutability.is_some()))
        .unwrap_or(false)
}

/// Build `let Self { field_a, field_b,.. } = self;`
fn build_destructure_stmt(field_kinds: &HashMap<String, FieldAccessKind>) -> syn::Stmt {
    use quote::quote;
    use syn::parse_quote;

    let mut sorted_fields: Vec<_> = field_kinds.iter().collect();
    sorted_fields.sort_by_key(|(name, _)| (*name).clone());

    let field_pats: Vec<proc_macro2::TokenStream> = sorted_fields
        .iter()
        .map(|(name, _kind)| {
            let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
            quote! { #ident }
        })
        .collect();

    parse_quote! {
        let Self { #(#field_pats),*, .. } = self;
    }
}

/// Replaces `self.field` expressions with bare `field` identifiers.
/// For assignment targets, inserts dereference since destructured fields are `&mut T`.
struct SelfFieldReplacer {
    field_kinds: HashMap<String, FieldAccessKind>,
}

impl VisitMut for SelfFieldReplacer {
    fn visit_expr_reference_mut(&mut self, reference: &mut syn::ExprReference) {
        if let syn::Expr::Field(field_expr) = reference.expr.as_ref() {
            if let Some(name) = extract_self_field_name(field_expr) {
                if self.field_kinds.contains_key(&name) {
                    *reference =
                        reference_to_destructured_field(&name, reference.mutability.is_some());
                    return;
                }
            }
        }

        syn::visit_mut::visit_expr_reference_mut(self, reference);
    }

    fn visit_expr_assign_mut(&mut self, assign: &mut syn::ExprAssign) {
        // Handle LHS: `self.field =...` → `*field =...`
        if let syn::Expr::Field(field_expr) = assign.left.as_ref() {
            if let Some(name) = extract_self_field_name(field_expr) {
                if self.field_kinds.contains_key(&name) {
                    *assign.left = deref_ident(&name);
                }
            }
        }
        // Visit RHS for reads
        self.visit_expr_mut(&mut assign.right);
    }

    fn visit_expr_binary_mut(&mut self, binary: &mut syn::ExprBinary) {
        // Handle compound assignment: `self.field +=...` → `*field +=...`
        if is_compound_assign(&binary.op) {
            if let syn::Expr::Field(field_expr) = binary.left.as_ref() {
                if let Some(name) = extract_self_field_name(field_expr) {
                    if self.field_kinds.contains_key(&name) {
                        *binary.left = deref_ident(&name);
                    }
                }
            }
        } else {
            self.rewrite_binary_operand(&mut binary.left);
        }
        self.rewrite_binary_operand(&mut binary.right);
    }

    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        // Check if this is a `self.field` expression for one of our fields.
        if let syn::Expr::Field(field_expr) = expr {
            if let Some(name) = extract_self_field_name(field_expr) {
                if self.field_kinds.contains_key(&name) {
                    let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
                    *expr = syn::Expr::Path(syn::ExprPath {
                        attrs: Vec::new(),
                        qself: None,
                        path: ident.into(),
                    });
                    return;
                }
            }
        }
        // Default traversal
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}

impl SelfFieldReplacer {
    fn rewrite_binary_operand(&mut self, expr: &mut syn::Expr) {
        if let syn::Expr::Field(field_expr) = expr {
            if let Some(name) = extract_self_field_name(field_expr) {
                if self.field_kinds.contains_key(&name) {
                    *expr = deref_ident(&name);
                    return;
                }
            }
        }

        self.visit_expr_mut(expr);
    }
}

/// Build `*field` deref expression from a field name.
fn deref_ident(name: &str) -> syn::Expr {
    let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
    let ident_expr = syn::Expr::Path(syn::ExprPath {
        attrs: Vec::new(),
        qself: None,
        path: ident.into(),
    });
    syn::Expr::Unary(syn::ExprUnary {
        attrs: Vec::new(),
        op: syn::UnOp::Deref(syn::token::Star::default()),
        expr: Box::new(ident_expr),
    })
}

fn reference_to_destructured_field(name: &str, mutable: bool) -> syn::ExprReference {
    syn::ExprReference {
        attrs: Vec::new(),
        and_token: syn::token::And::default(),
        mutability: mutable.then(syn::token::Mut::default),
        expr: Box::new(deref_ident(name)),
    }
}

/// Returns true if the binary op is a compound assignment (`+=`, `-=`, etc.).
fn is_compound_assign(op: &syn::BinOp) -> bool {
    matches!(
        op,
        syn::BinOp::AddAssign(_)
            | syn::BinOp::SubAssign(_)
            | syn::BinOp::MulAssign(_)
            | syn::BinOp::DivAssign(_)
            | syn::BinOp::RemAssign(_)
            | syn::BinOp::BitXorAssign(_)
            | syn::BinOp::BitAndAssign(_)
            | syn::BinOp::BitOrAssign(_)
            | syn::BinOp::ShlAssign(_)
            | syn::BinOp::ShrAssign(_)
    )
}

/// If `field_expr` is `self.<ident>`, return the ident name.
fn extract_self_field_name(field: &syn::ExprField) -> Option<String> {
    let syn::Expr::Path(path) = field.base.as_ref() else {
        return None;
    };
    if !path.path.is_ident("self") {
        return None;
    }
    match &field.member {
        syn::Member::Named(ident) => Some(ident.to_string()),
        syn::Member::Unnamed(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::generate_split_borrow;
    use kobo_analysis::split_borrow::detect_split_borrow_sites;

    fn apply_split_borrow(code: &str) -> String {
        let file = syn::parse_file(code).unwrap();
        let sites = detect_split_borrow_sites(&file.items);

        let mut file = file;
        for item in &mut file.items {
            let syn::Item::Impl(impl_block) = item else {
                continue;
            };
            for impl_item in &mut impl_block.items {
                let syn::ImplItem::Fn(method) = impl_item else {
                    continue;
                };
                let method_name = method.sig.ident.to_string();
                if let Some(site) = sites.iter().find(|s| s.method_name == method_name) {
                    generate_split_borrow(method, site);
                }
            }
        }

        prettyplease::unparse(&file)
    }

    #[test]
    fn test_generate_split_borrow_output() {
        let input = r#"
impl GameState {
    fn update(&mut self) {
        self.physics.step();
        self.renderer.draw(&self.world);
    }
}
"#;
        let output = apply_split_borrow(input);
        assert!(
            output.contains("let Self {"),
            "should contain destructure: {output}"
        );
        assert!(
            !output.contains("self.physics"),
            "self.physics should be replaced: {output}"
        );
        assert!(
            !output.contains("self.renderer"),
            "self.renderer should be replaced: {output}"
        );
        assert!(
            !output.contains("self.world"),
            "self.world should be replaced: {output}"
        );
    }

    #[test]
    fn test_split_borrow_preserves_immutable_self() {
        let input = r#"
impl State {
    fn display(&self) {
        let _ = self.name;
        let _ = self.count;
    }
}
"#;
        let output = apply_split_borrow(input);
        // &self methods should NOT be rewritten
        assert!(
            output.contains("self.name"),
            "should preserve self.name: {output}"
        );
        assert!(
            output.contains("self.count"),
            "should preserve self.count: {output}"
        );
    }

    #[test]
    fn test_split_borrow_fields_sorted() {
        let input = r#"
impl Game {
    fn tick(&mut self) {
        self.z_renderer.draw();
        self.a_physics.step();
    }
}
"#;
        let output = apply_split_borrow(input);
        // Fields should be sorted alphabetically in destructure
        let destructure_pos = output.find("let Self {").expect("missing destructure");
        let after = &output[destructure_pos..];
        let a_pos = after.find("a_physics").expect("missing a_physics");
        let z_pos = after.find("z_renderer").expect("missing z_renderer");
        assert!(a_pos < z_pos, "fields should be sorted: {output}");
    }
}
