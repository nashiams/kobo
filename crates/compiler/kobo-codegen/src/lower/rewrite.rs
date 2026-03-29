use kobo_ir::OwnershipTier;
use kobo_parser::KoboFile;
use quote::quote;
use syn::parse::Parser;
use syn::parse_quote;

use super::binding::{
    apply_tier_to_fn_arg_type, apply_tier_to_local, binding_for_pat, binding_tier_from_expr,
    is_mutating_method, wrapper_binding_from_expr,
};
use super::borrow_scope::{has_later_alias_use, rewritable_method_call, simple_borrow_alias};
use super::plan::{AnnotationNote, LoweringPlan};
use super::scope::ScopeStack;
use super::{LoweringAnchor, LoweringAnchorKind};

pub(crate) struct Lowerer<'a> {
    ast: &'a KoboFile,
    plan: &'a LoweringPlan,
    annotation_notes: Vec<AnnotationNote>,
    anchors: Vec<LoweringAnchor>,
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
        strip_kobo_attrs(&mut function.attrs);
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

    fn lower_block_statements(&mut self, block: &mut syn::Block, scopes: &mut ScopeStack) {
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

    fn lower_nested_block(&mut self, block: &mut syn::Block, scopes: &mut ScopeStack) {
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

        let replacement = build_borrow_scope_block_stmt(method_call, had_semi);
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

    fn lower_local(&mut self, local: &mut syn::Local, scopes: &mut ScopeStack) {
        strip_kobo_attrs(&mut local.attrs);
        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            self.lower_local_without_binding(local, scopes);
            return;
        };

        let tier = self.plan.tier_for_binding(binding);
        self.record_binding_anchor(binding, LoweringAnchorKind::Local);
        let already_wrapped = self.lower_local_initializer(local, binding, tier, scopes);
        apply_tier_to_local(local, tier, already_wrapped);
        scopes.insert(&binding.ident, tier);
    }

    fn lower_local_without_binding(&mut self, local: &mut syn::Local, scopes: &mut ScopeStack) {
        let Some(init) = &mut local.init else {
            return;
        };

        self.lower_expr(init.expr.as_mut(), scopes);
    }

    fn lower_local_initializer(
        &mut self,
        local: &mut syn::Local,
        binding: &kobo_parser::KoboBinding,
        target_tier: OwnershipTier,
        scopes: &mut ScopeStack,
    ) -> bool {
        let Some(init) = &mut local.init else {
            return false;
        };

        if self.plan.binding_uses_plain_clone_alias(binding) {
            if let Some((ident, _)) = binding_tier_from_expr(init.expr.as_ref(), scopes) {
                init.expr = Box::new(parse_quote!(#ident.clone()));
                return true;
            }
        }

        if let Some((ident, source_tier)) = binding_tier_from_expr(init.expr.as_ref(), scopes) {
            if source_tier.is_cloneable_wrapper() {
                init.expr = Box::new(parse_quote!(#ident.clone()));
                return true;
            }
            if source_tier == OwnershipTier::BoxOwned && target_tier == OwnershipTier::BoxOwned {
                return true;
            }
        }

        self.lower_expr(init.expr.as_mut(), scopes);
        false
    }

    fn lower_expr(&mut self, expr: &mut syn::Expr, scopes: &mut ScopeStack) {
        match expr {
            syn::Expr::Array(array) => self.lower_exprs(array.elems.iter_mut(), scopes),
            syn::Expr::Assign(assign) => {
                if let Some(replacement) = self.lower_assign_expr(assign, scopes) {
                    *expr = replacement;
                }
            }
            syn::Expr::Binary(binary) => {
                self.lower_expr(binary.left.as_mut(), scopes);
                self.lower_expr(binary.right.as_mut(), scopes);
            }
            syn::Expr::Block(block) => self.lower_nested_block(&mut block.block, scopes),
            syn::Expr::Call(call) => self.lower_call_expr(call, scopes),
            syn::Expr::Cast(cast) => self.lower_expr(cast.expr.as_mut(), scopes),
            syn::Expr::Field(field) => self.lower_expr(field.base.as_mut(), scopes),
            syn::Expr::ForLoop(for_loop) => self.lower_for_loop_expr(for_loop, scopes),
            syn::Expr::Group(group) => self.lower_expr(group.expr.as_mut(), scopes),
            syn::Expr::If(expr_if) => self.lower_if_expr(expr_if, scopes),
            syn::Expr::Index(index) => {
                self.lower_expr(index.expr.as_mut(), scopes);
                self.lower_expr(index.index.as_mut(), scopes);
            }
            syn::Expr::Loop(expr_loop) => self.lower_nested_block(&mut expr_loop.body, scopes),
            syn::Expr::Macro(expr_macro) => {
                self.lower_macro_tokens(&mut expr_macro.mac.tokens, scopes);
            }
            syn::Expr::Match(expr_match) => self.lower_match_expr(expr_match, scopes),
            syn::Expr::MethodCall(method_call) => self.lower_method_call_expr(method_call, scopes),
            syn::Expr::Paren(paren) => self.lower_expr(paren.expr.as_mut(), scopes),
            syn::Expr::Path(path) => {
                if let Some(replacement) = self.lower_path_expr(path, scopes) {
                    *expr = replacement;
                }
            }
            syn::Expr::Reference(reference) => {
                if let Some(replacement) = self.lower_reference_expr(reference, scopes) {
                    *expr = replacement;
                }
            }
            syn::Expr::Repeat(repeat) => {
                self.lower_expr(repeat.expr.as_mut(), scopes);
                self.lower_expr(repeat.len.as_mut(), scopes);
            }
            syn::Expr::Return(return_expr) => self.lower_return_expr(return_expr, scopes),
            syn::Expr::Struct(expr_struct) => self.lower_struct_expr(expr_struct, scopes),
            syn::Expr::Tuple(tuple) => self.lower_exprs(tuple.elems.iter_mut(), scopes),
            syn::Expr::Unary(unary) => self.lower_expr(unary.expr.as_mut(), scopes),
            syn::Expr::While(expr_while) => self.lower_while_expr(expr_while, scopes),
            _ => {}
        }
    }

    fn lower_exprs<'expr>(
        &mut self,
        exprs: impl Iterator<Item = &'expr mut syn::Expr>,
        scopes: &mut ScopeStack,
    ) {
        for expr in exprs {
            self.lower_expr(expr, scopes);
        }
    }

    fn lower_assign_expr(
        &mut self,
        assign: &mut syn::ExprAssign,
        scopes: &mut ScopeStack,
    ) -> Option<syn::Expr> {
        let Some((ident, tier)) = wrapper_binding_from_expr(assign.left.as_ref(), scopes) else {
            self.lower_expr(assign.left.as_mut(), scopes);
            self.lower_expr(assign.right.as_mut(), scopes);
            return None;
        };

        self.lower_expr(assign.right.as_mut(), scopes);
        let right = (*assign.right).clone();
        match tier {
            OwnershipTier::RcMutShared => Some(parse_quote!(*#ident.borrow_mut() = #right)),
            _ => None,
        }
    }

    fn lower_call_expr(&mut self, call: &mut syn::ExprCall, scopes: &mut ScopeStack) {
        self.lower_expr(call.func.as_mut(), scopes);
        let parameter_tiers = self.plan.called_function_param_tiers(call.func.as_ref());

        for (index, argument) in call.args.iter_mut().enumerate() {
            let expected_tier = parameter_tiers.and_then(|tiers| tiers.get(index)).copied();
            if let Some(expected_tier) = expected_tier {
                if let Some((ident, source_tier)) = binding_tier_from_expr(argument, scopes) {
                    if source_tier == expected_tier {
                        if expected_tier.is_cloneable_wrapper() {
                            *argument = parse_quote!(#ident.clone());
                        }
                        continue;
                    }
                }

                self.lower_expr(argument, scopes);
                if should_wrap_argument(expected_tier) {
                    let lowered = (*argument).clone();
                    *argument = wrap_argument_expr(lowered, expected_tier);
                }
                continue;
            }

            self.lower_expr(argument, scopes);
        }
    }

    fn lower_for_loop_expr(&mut self, for_loop: &mut syn::ExprForLoop, scopes: &mut ScopeStack) {
        self.lower_expr(for_loop.expr.as_mut(), scopes);
        self.lower_nested_block(&mut for_loop.body, scopes);
    }

    fn lower_if_expr(&mut self, expr_if: &mut syn::ExprIf, scopes: &mut ScopeStack) {
        self.lower_expr(expr_if.cond.as_mut(), scopes);
        self.lower_nested_block(&mut expr_if.then_branch, scopes);

        if let Some((_, else_branch)) = &mut expr_if.else_branch {
            self.lower_expr(else_branch.as_mut(), scopes);
        }
    }

    fn lower_match_expr(&mut self, expr_match: &mut syn::ExprMatch, scopes: &mut ScopeStack) {
        self.lower_expr(expr_match.expr.as_mut(), scopes);

        for arm in &mut expr_match.arms {
            if let Some((_, guard)) = &mut arm.guard {
                self.lower_expr(guard.as_mut(), scopes);
            }

            self.lower_expr(arm.body.as_mut(), scopes);
        }
    }

    fn lower_method_call_expr(
        &mut self,
        method_call: &mut syn::ExprMethodCall,
        scopes: &mut ScopeStack,
    ) {
        self.lower_method_receiver(method_call, scopes);
        self.lower_exprs(method_call.args.iter_mut(), scopes);
    }

    fn lower_method_receiver(
        &mut self,
        method_call: &mut syn::ExprMethodCall,
        scopes: &mut ScopeStack,
    ) {
        let Some((ident, tier)) = wrapper_binding_from_expr(method_call.receiver.as_ref(), scopes)
        else {
            self.lower_expr(method_call.receiver.as_mut(), scopes);
            return;
        };

        if tier == OwnershipTier::RcMutShared {
            method_call.receiver = Box::new(lowered_receiver_expr(ident, &method_call.method));
            return;
        }

        self.lower_expr(method_call.receiver.as_mut(), scopes);
    }

    fn lower_path_expr(
        &mut self,
        path: &mut syn::ExprPath,
        scopes: &mut ScopeStack,
    ) -> Option<syn::Expr> {
        let Some((ident, tier)) = wrapper_binding_from_expr(&syn::Expr::Path(path.clone()), scopes)
        else {
            return None;
        };

        (tier == OwnershipTier::RcMutShared).then(|| parse_quote!(#ident.borrow()))
    }

    fn lower_reference_expr(
        &mut self,
        reference: &mut syn::ExprReference,
        scopes: &mut ScopeStack,
    ) -> Option<syn::Expr> {
        let Some((ident, tier)) = wrapper_binding_from_expr(reference.expr.as_ref(), scopes) else {
            self.lower_expr(reference.expr.as_mut(), scopes);
            return None;
        };

        match (tier, reference.mutability.is_some()) {
            (OwnershipTier::RcMutShared, true) => Some(parse_quote!(&mut *#ident.borrow_mut())),
            (OwnershipTier::RcMutShared, false) => Some(parse_quote!(&*#ident.borrow())),
            _ => None,
        }
    }

    fn lower_return_expr(&mut self, return_expr: &mut syn::ExprReturn, scopes: &mut ScopeStack) {
        if let Some(inner) = &mut return_expr.expr {
            self.lower_expr(inner.as_mut(), scopes);
        }
    }

    fn lower_struct_expr(&mut self, expr_struct: &mut syn::ExprStruct, scopes: &mut ScopeStack) {
        for field in &mut expr_struct.fields {
            self.lower_expr(&mut field.expr, scopes);
        }

        if let Some(rest) = &mut expr_struct.rest {
            self.lower_expr(rest.as_mut(), scopes);
        }
    }

    fn lower_while_expr(&mut self, expr_while: &mut syn::ExprWhile, scopes: &mut ScopeStack) {
        self.lower_expr(expr_while.cond.as_mut(), scopes);
        self.lower_nested_block(&mut expr_while.body, scopes);
    }

    fn lower_macro_tokens(
        &mut self,
        tokens: &mut proc_macro2::TokenStream,
        scopes: &mut ScopeStack,
    ) {
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        let Ok(mut exprs) = parser.parse2(tokens.clone()) else {
            return;
        };

        self.lower_exprs(exprs.iter_mut(), scopes);
        *tokens = quote!(#exprs);
    }

    fn record_binding_anchor(
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

fn lowered_receiver_expr(ident: syn::Ident, method: &syn::Ident) -> syn::Expr {
    if is_mutating_method(method) {
        parse_quote!(#ident.borrow_mut())
    } else {
        parse_quote!(#ident.borrow())
    }
}

fn should_wrap_argument(tier: OwnershipTier) -> bool {
    matches!(
        tier,
        OwnershipTier::BoxOwned
            | OwnershipTier::RcShared
            | OwnershipTier::ArcShared
            | OwnershipTier::RcMutShared
            | OwnershipTier::Scoped
    )
}

fn wrap_argument_expr(expr: syn::Expr, tier: OwnershipTier) -> syn::Expr {
    match tier {
        OwnershipTier::BoxOwned => parse_quote!(Box::new(#expr)),
        OwnershipTier::RcShared => parse_quote!(Rc::new(#expr)),
        OwnershipTier::ArcShared => parse_quote!(Arc::new(#expr)),
        OwnershipTier::RcMutShared => parse_quote!(Rc::new(RefCell::new(#expr))),
        OwnershipTier::Scoped => parse_quote!(ScopedHandle::new(#expr)),
        _ => expr,
    }
}

fn build_borrow_scope_block_stmt(method_call: syn::ExprMethodCall, had_semi: bool) -> syn::Stmt {
    let method_expr = syn::Expr::MethodCall(method_call);
    if had_semi {
        parse_quote!({
            #method_expr;
        })
    } else {
        parse_quote!({
            #method_expr
        })
    }
}

fn strip_kobo_attrs(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|attr| {
        !attr
            .path()
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "kobo")
    });
}
