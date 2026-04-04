mod expr;
mod local;
mod util;

use kobo_parser::KoboFile;
use syn::parse_quote;

use super::binding::{apply_tier_to_fn_arg_type, binding_for_pat};
use super::borrow_scope::{has_later_alias_use, rewritable_method_call, simple_borrow_alias};
use super::plan::{AnnotationNote, LoweringPlan};
use super::scope::ScopeStack;
use super::{LoweringAnchor, LoweringAnchorKind};

pub(crate) struct Lowerer<'a> {
    pub(super) ast: &'a KoboFile,
    pub(super) plan: &'a LoweringPlan,
    pub(super) annotation_notes: Vec<AnnotationNote>,
    pub(super) anchors: Vec<LoweringAnchor>,
}

impl<'a> Lowerer<'a> {
    pub(crate) fn new(ast: &'a KoboFile, plan: &'a LoweringPlan) -> Self {
        Self {
            ast,
            plan,
            annotation_notes: Vec::new(),
            anchors: Vec::new(),
        }
    }

    pub(crate) fn lower_items(&mut self, items: &mut [syn::Item]) {
        for item in items {
            self.lower_item(item);
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<AnnotationNote>, Vec<LoweringAnchor>) {
        (self.annotation_notes, self.anchors)
    }

    fn lower_item(&mut self, item: &mut syn::Item) {
        match item {
            syn::Item::Fn(function) => self.lower_function(function),
            syn::Item::Const(item_const) => {
                self.record_item_anchor(&item_const.ident, LoweringAnchorKind::Const)
            }
            syn::Item::Static(item_static) => {
                self.record_item_anchor(&item_static.ident, LoweringAnchorKind::Static)
            }
            _ => {}
        }
    }

    fn lower_function(&mut self, function: &mut syn::ItemFn) {
        util::strip_kobo_attrs(&mut function.attrs);
        let mut scopes = ScopeStack::new();
        scopes.push();
        self.lower_function_params(&mut function.sig.inputs, &mut scopes);
        self.lower_block_statements(&mut function.block, &mut scopes);
        scopes.pop();
    }

    fn lower_function_params(
        &mut self,
        inputs: &mut syn::punctuated::Punctuated<syn::FnArg, syn::Token![,]>,
        scopes: &mut ScopeStack,
    ) {
        for input in inputs {
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            let Some(binding) = binding_for_pat(self.ast, &argument.pat) else {
                continue;
            };

            let tier = self.plan.tier_for_binding(binding);
            self.record_binding_anchor(binding, LoweringAnchorKind::Parameter);
            apply_tier_to_fn_arg_type(argument, tier);
            scopes.insert(&binding.ident, tier);
        }
    }

    pub(super) fn lower_block_statements(
        &mut self,
        block: &mut syn::Block,
        scopes: &mut ScopeStack,
    ) {
        let mut index = 0usize;
        while index < block.stmts.len() {
            if self.try_shrink_borrow_alias(block, index, scopes) {
                index += 1;
                continue;
            }

            self.lower_stmt(&mut block.stmts[index], scopes);
            index += 1;
        }
    }

    pub(super) fn lower_nested_block(
        &mut self,
        block: &mut syn::Block,
        scopes: &mut ScopeStack,
    ) {
        scopes.push();
        self.lower_block_statements(block, scopes);
        scopes.pop();
    }

    fn lower_stmt(&mut self, stmt: &mut syn::Stmt, scopes: &mut ScopeStack) {
        match stmt {
            syn::Stmt::Local(local) => self.lower_local(local, scopes),
            syn::Stmt::Item(item) => self.lower_item(item),
            syn::Stmt::Expr(expr, _) => self.lower_expr(expr, scopes),
            syn::Stmt::Macro(stmt_macro) => {
                self.lower_macro_tokens(&mut stmt_macro.mac.tokens, scopes);
            }
        }
    }

    fn try_shrink_borrow_alias(
        &mut self,
        block: &mut syn::Block,
        index: usize,
        scopes: &mut ScopeStack,
    ) -> bool {
        let Some(syn::Stmt::Local(local)) = block.stmts.get(index) else {
            return false;
        };
        let Some(alias) = simple_borrow_alias(local, scopes) else {
            return false;
        };
        let Some(next_stmt) = block.stmts.get(index + 1) else {
            self.record_borrow_scope_conservative(local);
            return false;
        };
        if has_later_alias_use(&block.stmts[index + 2..], &alias.alias_ident) {
            self.record_borrow_scope_conservative(local);
            return false;
        }

        let Some((mut method_call, had_semi)) =
            rewritable_method_call(next_stmt, &alias.alias_ident)
        else {
            self.record_borrow_scope_conservative(local);
            return false;
        };

        for argument in &mut method_call.args {
            self.lower_expr(argument, scopes);
        }

        let source_ident = alias.source_ident;
        method_call.receiver = if alias.is_mutable {
            Box::new(parse_quote!(#source_ident.borrow_mut()))
        } else {
            Box::new(parse_quote!(#source_ident.borrow()))
        };

        let replacement = util::build_borrow_scope_block_stmt(method_call, had_semi);
        block.stmts[index] = replacement;
        block.stmts.remove(index + 1);
        true
    }

    fn record_borrow_scope_conservative(&mut self, local: &syn::Local) {
        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            return;
        };
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        let (kobo_line, _) = self.ast.line_col(binding.span);
        if self
            .annotation_notes
            .iter()
            .any(|note| note.node == node && note.reason == "borrow-scope-conservative")
        {
            return;
        }

        self.annotation_notes.push(AnnotationNote {
            node,
            binding_name: binding.ident.to_string(),
            kobo_line,
            reason: "borrow-scope-conservative".to_owned(),
        });
    }

    pub(super) fn record_binding_anchor(
        &mut self,
        binding: &kobo_parser::KoboBinding,
        kind: LoweringAnchorKind,
    ) {
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        self.anchors.push(LoweringAnchor { node, kind });
    }

    fn record_item_anchor(&mut self, ident: &syn::Ident, kind: LoweringAnchorKind) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span) else {
            return;
        };
        self.record_binding_anchor(binding, kind);
    }
}
