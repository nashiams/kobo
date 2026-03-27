use kobo_ir::OwnershipTier;
use kobo_parser::KoboFile;
use quote::quote;
use syn::parse::Parser;
use syn::parse_quote;

use super::binding::{
    apply_tier_to_fn_arg_type, apply_tier_to_local, binding_for_pat, is_mutating_method,
    wrapped_ident_from_expr,
};
use super::plan::LoweringPlan;
use super::scope::ScopeStack;

pub(crate) struct Lowerer<'a> {
    ast: &'a KoboFile,
    plan: &'a LoweringPlan,
}

impl<'a> Lowerer<'a> {
    pub(crate) fn new(ast: &'a KoboFile, plan: &'a LoweringPlan) -> Self {
        Self { ast, plan }
    }

    pub(crate) fn lower_items(&mut self, items: &mut [syn::Item]) {
        for item in items {
            self.lower_item(item);
        }
    }

    fn lower_item(&mut self, item: &mut syn::Item) {
        if let syn::Item::Fn(function) = item {
            self.lower_function(function);
        }
    }

    fn lower_function(&mut self, function: &mut syn::ItemFn) {
        let mut scopes = ScopeStack::new();
        scopes.push();
        self.lower_function_params(&mut function.sig.inputs, &mut scopes);
        self.lower_block_statements(&mut function.block, &mut scopes);
        scopes.pop();
    }

    fn lower_function_params(
        &self,
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
            apply_tier_to_fn_arg_type(argument, tier);
            scopes.insert(&binding.ident, tier);
        }
    }

    fn lower_block_statements(&mut self, block: &mut syn::Block, scopes: &mut ScopeStack) {
        for stmt in &mut block.stmts {
            self.lower_stmt(stmt, scopes);
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

    fn lower_local(&mut self, local: &mut syn::Local, scopes: &mut ScopeStack) {
        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            self.lower_local_without_binding(local, scopes);
            return;
        };

        let tier = self.plan.tier_for_binding(binding);
        let already_wrapped = self.lower_local_initializer(local, tier, scopes);
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
        target_tier: OwnershipTier,
        scopes: &mut ScopeStack,
    ) -> bool {
        let Some(init) = &mut local.init else {
            return false;
        };

        if target_tier == OwnershipTier::RcMutShared {
            if let Some(ident) = wrapped_ident_from_expr(init.expr.as_ref(), scopes) {
                init.expr = Box::new(parse_quote!(#ident.clone()));
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
            syn::Expr::MethodCall(method_call) => {
                self.lower_method_call_expr(method_call, scopes);
            }
            syn::Expr::Paren(paren) => self.lower_expr(paren.expr.as_mut(), scopes),
            syn::Expr::Path(path) => {
                if let Some(replacement) = self.lower_path_expr(path, scopes) {
                    *expr = replacement;
                }
            }
            syn::Expr::Reference(reference) => self.lower_expr(reference.expr.as_mut(), scopes),
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
        let Some(ident) = wrapped_ident_from_expr(assign.left.as_ref(), scopes) else {
            self.lower_expr(assign.left.as_mut(), scopes);
            self.lower_expr(assign.right.as_mut(), scopes);
            return None;
        };

        self.lower_expr(assign.right.as_mut(), scopes);
        let right = (*assign.right).clone();
        Some(parse_quote!(*#ident.borrow_mut() = #right))
    }

    fn lower_call_expr(&mut self, call: &mut syn::ExprCall, scopes: &mut ScopeStack) {
        self.lower_expr(call.func.as_mut(), scopes);
        let parameter_tiers = self.plan.called_function_param_tiers(call.func.as_ref());

        for (index, argument) in call.args.iter_mut().enumerate() {
            let expects_wrapped = parameter_tiers.and_then(|tiers| tiers.get(index)).copied()
                == Some(OwnershipTier::RcMutShared);

            if expects_wrapped {
                if let Some(ident) = wrapped_ident_from_expr(argument, scopes) {
                    *argument = parse_quote!(#ident.clone());
                    continue;
                }
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
        let Some(ident) = wrapped_ident_from_expr(method_call.receiver.as_ref(), scopes) else {
            self.lower_expr(method_call.receiver.as_mut(), scopes);
            return;
        };

        method_call.receiver = Box::new(lowered_receiver_expr(ident, &method_call.method));
    }

    fn lower_path_expr(
        &mut self,
        path: &mut syn::ExprPath,
        scopes: &mut ScopeStack,
    ) -> Option<syn::Expr> {
        let Some(ident) = wrapped_ident_from_expr(&syn::Expr::Path(path.clone()), scopes) else {
            return None;
        };

        Some(parse_quote!(#ident.borrow()))
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
}

fn lowered_receiver_expr(ident: syn::Ident, method: &syn::Ident) -> syn::Expr {
    if is_mutating_method(method) {
        parse_quote!(#ident.borrow_mut())
    } else {
        parse_quote!(#ident.borrow())
    }
}
