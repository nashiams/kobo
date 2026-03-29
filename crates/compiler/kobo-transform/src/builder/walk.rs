use syn::spanned::Spanned;

use crate::hint::parse_hint;
use kobo_ir::{EscapeKind, UseKind};

use super::helpers::{assignment_escape_kind, borrow_kind, is_mutating_method};
use super::{FunctionCtx, PendingHint, TransformFactsBuilder};

impl TransformFactsBuilder<'_> {
    pub(super) fn walk_item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Fn(function) => self.walk_function(function),
            syn::Item::Const(item_const) => {
                self.emit_declared_binding(&item_const.ident, Some(&item_const.expr), None);
            }
            syn::Item::Static(item_static) => {
                self.emit_declared_binding(&item_static.ident, Some(&item_static.expr), None);
            }
            _ => {}
        }
    }

    fn walk_function(&mut self, function: &syn::ItemFn) {
        let generic_params = function
            .sig
            .generics
            .params
            .iter()
            .filter_map(|param| match param {
                syn::GenericParam::Type(ty) => Some(ty.ident.to_string()),
                _ => None,
            })
            .collect();
        let function_hint = parse_hint(&function.attrs).map(|(hint, span)| PendingHint {
            hint,
            span: self.ast.span_from_syn(span),
        });
        self.function_stack.push(FunctionCtx {
            generic_params,
            is_async: function.sig.asyncness.is_some(),
            hint: function_hint,
            hint_consumed: false,
        });

        self.enter_scope(self.ast.span_from_syn(function.span()));
        self.ctx.push_scope();

        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                let param_hint = self.take_function_hint();
                self.emit_pat_binding(&argument.pat, None, param_hint);
            }
        }

        self.walk_block_statements(&function.block.stmts);

        self.ctx.pop_scope();
        self.exit_scope(self.ast.span_from_syn(function.span()));
        self.function_stack.pop();
    }

    fn walk_block(&mut self, block: &syn::Block) {
        self.enter_scope(self.ast.span_from_syn(block.span()));
        self.ctx.push_scope();
        self.walk_block_statements(&block.stmts);
        self.ctx.pop_scope();
        self.exit_scope(self.ast.span_from_syn(block.span()));
    }

    fn walk_block_statements(&mut self, statements: &[syn::Stmt]) {
        for stmt in statements {
            self.walk_stmt(stmt);
        }
    }

    fn walk_stmt(&mut self, stmt: &syn::Stmt) {
        match stmt {
            syn::Stmt::Local(local) => self.walk_local(local),
            syn::Stmt::Item(item) => self.walk_item(item),
            syn::Stmt::Expr(expr, _) => self.walk_expr(expr),
            syn::Stmt::Macro(_) => {}
        }
    }

    fn walk_local(&mut self, local: &syn::Local) {
        let init = local.init.as_ref().map(|init| init.expr.as_ref());
        let Some(binding) = self.binding_for_pat(&local.pat).cloned() else {
            if let Some(init_expr) = init {
                self.walk_expr(init_expr);
            }
            return;
        };

        let local_hint = parse_hint(&local.attrs).map(|(hint, span)| PendingHint {
            hint,
            span: self.ast.span_from_syn(span),
        });

        let binding_state = self.emit_binding(&binding, init, local_hint);
        if let Some(init_expr) = init {
            if let Some((mut move_alias, source_state)) =
                self.move_alias_from_initializer(binding_state.decl_id, init_expr)
            {
                let alias_is_candidate = self.pending_elision_candidates.contains_key(&binding.id);
                self.record_pending_elision_candidate(
                    &binding,
                    binding_state.decl_id,
                    source_state.decl_id,
                );
                self.maybe_mark_plain_clone_alias(
                    binding_state.decl_id,
                    source_state,
                    move_alias.span,
                    alias_is_candidate,
                );
                move_alias.plain_clone_alias &= !alias_is_candidate;
                self.push_move_alias(move_alias);
            }
            if let Some(alias) =
                self.borrow_alias_from_initializer(binding_state.decl_id, init_expr)
            {
                self.push_borrow_alias(alias);
            }
            self.walk_local_initializer(init_expr);
        }
        self.ctx.define(&binding.ident, binding_state);
    }

    fn walk_expr(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Array(array) => {
                for element in &array.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Assign(assign) => {
                let escape_kind = assignment_escape_kind(assign.left.as_ref());
                self.walk_assignment_target(assign.left.as_ref());
                self.walk_move_context_expr(assign.right.as_ref(), escape_kind);
            }
            syn::Expr::Binary(binary) => {
                self.walk_expr(binary.left.as_ref());
                self.walk_expr(binary.right.as_ref());
            }
            syn::Expr::Block(block) => self.walk_block(&block.block),
            syn::Expr::Call(call) => self.walk_call(call),
            syn::Expr::Cast(cast) => self.walk_expr(cast.expr.as_ref()),
            syn::Expr::Closure(closure) => self.walk_expr(closure.body.as_ref()),
            syn::Expr::Field(field) => self.walk_expr(field.base.as_ref()),
            syn::Expr::ForLoop(for_loop) => {
                self.walk_expr(for_loop.expr.as_ref());
                self.walk_block(&for_loop.body);
            }
            syn::Expr::Group(group) => self.walk_expr(group.expr.as_ref()),
            syn::Expr::If(expr_if) => {
                self.walk_expr(expr_if.cond.as_ref());
                self.walk_block(&expr_if.then_branch);
                if let Some((_, else_branch)) = &expr_if.else_branch {
                    self.walk_expr(else_branch.as_ref());
                }
            }
            syn::Expr::Index(index) => {
                self.walk_expr(index.expr.as_ref());
                self.walk_expr(index.index.as_ref());
            }
            syn::Expr::Loop(expr_loop) => self.walk_block(&expr_loop.body),
            syn::Expr::Macro(_) => {}
            syn::Expr::Match(expr_match) => {
                self.walk_expr(expr_match.expr.as_ref());
                for arm in &expr_match.arms {
                    if let Some((_, guard)) = &arm.guard {
                        self.walk_expr(guard.as_ref());
                    }
                    self.walk_expr(arm.body.as_ref());
                }
            }
            syn::Expr::MethodCall(method_call) => {
                self.walk_method_receiver(method_call.receiver.as_ref(), &method_call.method);
                for argument in &method_call.args {
                    self.walk_expr(argument);
                }
            }
            syn::Expr::Paren(paren) => self.walk_expr(paren.expr.as_ref()),
            syn::Expr::Path(path) => self.walk_path_expr(path),
            syn::Expr::Reference(reference) => self.walk_reference_expr(reference),
            syn::Expr::Repeat(repeat) => {
                self.walk_expr(repeat.expr.as_ref());
                self.walk_expr(repeat.len.as_ref());
            }
            syn::Expr::Return(return_expr) => {
                if let Some(expr) = &return_expr.expr {
                    self.walk_move_context_expr(
                        expr.as_ref(),
                        Some(EscapeKind::ReturnedFromFunction),
                    );
                }
            }
            syn::Expr::Struct(expr_struct) => {
                for field in &expr_struct.fields {
                    self.walk_move_context_expr(&field.expr, Some(EscapeKind::StoredInStruct));
                }
                if let Some(rest) = &expr_struct.rest {
                    self.walk_expr(rest.as_ref());
                }
            }
            syn::Expr::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Unary(unary) => self.walk_expr(unary.expr.as_ref()),
            syn::Expr::While(expr_while) => {
                self.walk_expr(expr_while.cond.as_ref());
                self.walk_block(&expr_while.body);
            }
            _ => {}
        }
    }

    fn walk_call(&mut self, call: &syn::ExprCall) {
        self.walk_expr(call.func.as_ref());
        let escape_kind = if self.is_local_function_expr(call.func.as_ref()) {
            None
        } else {
            Some(EscapeKind::PassedToOpaqueCall)
        };
        for argument in &call.args {
            let syn::Expr::Reference(reference) = argument else {
                self.walk_move_context_expr(argument, escape_kind);
                continue;
            };

            if self.emit_ephemeral_borrow(reference.expr.as_ref(), borrow_kind(reference)) {
                continue;
            }

            self.walk_expr(argument);
        }
    }

    pub(super) fn walk_local_initializer(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Reference(reference) => self.walk_reference_expr(reference),
            _ => self.walk_move_context_expr(expr, None),
        }
    }

    fn walk_move_context_expr(&mut self, expr: &syn::Expr, escape_kind: Option<EscapeKind>) {
        if self.emit_move(expr, escape_kind) {
            return;
        }

        self.walk_expr(expr);
    }

    fn walk_assignment_target(&mut self, expr: &syn::Expr) {
        if self.emit_use(expr, UseKind::Write) {
            return;
        }

        self.walk_expr(expr);
    }

    fn walk_method_receiver(&mut self, receiver: &syn::Expr, method: &syn::Ident) {
        let use_kind = if is_mutating_method(method) {
            UseKind::Write
        } else {
            UseKind::Read
        };

        if self.emit_use(receiver, use_kind) {
            return;
        }

        self.walk_expr(receiver);
    }

    fn walk_path_expr(&mut self, path: &syn::ExprPath) {
        let _ = self.emit_use(&syn::Expr::Path(path.clone()), UseKind::Read);
    }

    fn walk_reference_expr(&mut self, reference: &syn::ExprReference) {
        if self.emit_borrow(reference.expr.as_ref(), borrow_kind(reference)) {
            return;
        }

        self.walk_expr(reference.expr.as_ref());
    }
}
