use syn::spanned::Spanned;

use crate::hint::parse_hint;
use kobo_ir::{EscapeKind, KoboSpan, MigrateSite, MigrateTarget, RelaxAttrError, UseKind};
use proc_macro2::{TokenStream, TokenTree};

use super::attrs::{
    parse_async_shared_attr, parse_migrate_attr, parse_relax_attr, AsyncSharedAttrResult,
    MigrateAttrResult, RelaxAttrResult, SharedAttrResult,
};
use super::helpers::{assignment_escape_kind, borrow_kind, is_mutating_method};
use super::{FunctionCtx, PendingHint, TransformFactsBuilder};

impl TransformFactsBuilder<'_> {
    pub(super) fn walk_item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Fn(function) => self.walk_function(function),
            syn::Item::Struct(item_struct) => {
                self.check_relax_on_non_fn_item(&item_struct.attrs, item_struct.span());
                self.collect_struct_def(item_struct);
            }
            syn::Item::Const(item_const) => {
                self.check_relax_on_non_fn_item(&item_const.attrs, item_const.span());
                self.emit_declared_binding(&item_const.ident, Some(&item_const.expr), None);
            }
            syn::Item::Static(item_static) => {
                self.check_relax_on_non_fn_item(&item_static.attrs, item_static.span());
                self.emit_declared_binding(&item_static.ident, Some(&item_static.expr), None);
            }
            syn::Item::Impl(item_impl) => {
                self.check_relax_on_non_fn_item(&item_impl.attrs, item_impl.span());
                self.walk_impl_block(item_impl);
            }
            item => {
                // Catch #[kobo::relax] on enum, mod, trait, etc.
                let attrs = match item {
                    syn::Item::Enum(i) => Some(i.attrs.as_slice()),
                    syn::Item::Mod(i) => Some(i.attrs.as_slice()),
                    syn::Item::Trait(i) => Some(i.attrs.as_slice()),
                    _ => None,
                };
                if let Some(attrs) = attrs {
                    self.check_relax_on_non_fn_item(attrs, item.span());
                }
            }
        }
    }

    /// Emit a `RelaxAttrError` for each `#[kobo::relax]` found on a non-function item.
    fn check_relax_on_non_fn_item(
        &mut self,
        attrs: &[syn::Attribute],
        item_span: proc_macro2::Span,
    ) {
        for attr in attrs {
            match parse_relax_attr(attr) {
                RelaxAttrResult::Valid(attr_span) | RelaxAttrResult::HasArguments(attr_span) => {
                    let span = self.ast.span_from_syn(attr_span);
                    let _ = item_span; // keep span tight to the attribute
                    self.relax_attr_errors.push(RelaxAttrError {
                        span,
                        message: "`#[kobo::relax]` can only be applied to functions".to_owned(),
                        is_error: true,
                    });
                }
                RelaxAttrResult::NotRelax => {}
            }
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

        let fn_span = self.ast.span_from_syn(function.span());
        let relax_count = self.collect_relax_attrs_on_fn(&function.attrs, fn_span);
        self.check_relax_strict_conflict(relax_count, fn_span);
        self.collect_migrate_attrs(&function.attrs, MigrateTarget::Function);

        self.function_stack.push(FunctionCtx {
            generic_params,
            is_async: function.sig.asyncness.is_some(),
            hint: function_hint,
            hint_consumed: false,
        });

        self.enter_scope(fn_span);
        self.ctx.push_scope();

        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                self.collect_migrate_attrs(&argument.attrs, MigrateTarget::Parameter);
                let param_hint = self.take_function_hint();
                self.emit_pat_binding(&argument.pat, None, param_hint);
            }
        }

        self.walk_block_statements(&function.block.stmts);

        self.ctx.pop_scope();
        self.exit_scope(fn_span);
        self.function_stack.pop();
    }

    fn walk_impl_block(&mut self, item_impl: &syn::ItemImpl) {
        for impl_item in &item_impl.items {
            if let syn::ImplItem::Fn(method) = impl_item {
                self.walk_impl_method(method);
            }
        }
    }

    fn walk_impl_method(&mut self, method: &syn::ImplItemFn) {
        let generic_params = method
            .sig
            .generics
            .params
            .iter()
            .filter_map(|param| match param {
                syn::GenericParam::Type(ty) => Some(ty.ident.to_string()),
                _ => None,
            })
            .collect();
        let function_hint = parse_hint(&method.attrs).map(|(hint, span)| PendingHint {
            hint,
            span: self.ast.span_from_syn(span),
        });

        let fn_span = self.ast.span_from_syn(method.span());
        let relax_count = self.collect_relax_attrs_on_fn(&method.attrs, fn_span);
        self.check_relax_strict_conflict(relax_count, fn_span);
        self.collect_migrate_attrs(&method.attrs, MigrateTarget::Function);

        self.function_stack.push(FunctionCtx {
            generic_params,
            is_async: method.sig.asyncness.is_some(),
            hint: function_hint,
            hint_consumed: false,
        });

        self.enter_scope(fn_span);
        self.ctx.push_scope();

        for input in &method.sig.inputs {
            match input {
                syn::FnArg::Receiver(_) => {
                    // &self / &mut self — no ownership tracking needed.
                }
                syn::FnArg::Typed(argument) => {
                    self.collect_migrate_attrs(&argument.attrs, MigrateTarget::Parameter);
                    let param_hint = self.take_function_hint();
                    self.emit_pat_binding(&argument.pat, None, param_hint);
                }
            }
        }

        self.walk_block_statements(&method.block.stmts);

        self.ctx.pop_scope();
        self.exit_scope(fn_span);
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
            syn::Stmt::Macro(stmt_macro) => self.walk_macro_stmt(&stmt_macro.mac),
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

        self.collect_migrate_attrs(&local.attrs, MigrateTarget::LetBinding);

        // Detect explicit shared-state opt-ins on let bindings.
        for attr in &local.attrs {
            if let AsyncSharedAttrResult::Valid | AsyncSharedAttrResult::HasArguments =
                parse_async_shared_attr(attr)
            {
                self.pending_async_shared = true;
            }
            if let SharedAttrResult::Valid | SharedAttrResult::HasArguments =
                super::attrs::parse_shared_attr(attr)
            {
                self.pending_async_shared = true;
            }
        }

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
            syn::Expr::Macro(expr_macro) => self.walk_macro_stmt(&expr_macro.mac),
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
        let is_async_local_call = self.is_async_local_function_expr(call.func.as_ref());
        self.walk_expr(call.func.as_ref());
        let escape_kind = if self.is_local_function_expr(call.func.as_ref()) {
            None
        } else {
            Some(EscapeKind::PassedToOpaqueCall)
        };
        for argument in &call.args {
            let syn::Expr::Reference(reference) = argument else {
                if is_async_local_call {
                    self.mark_expr_needs_send(argument);
                }
                self.walk_move_context_expr(argument, escape_kind);
                continue;
            };

            if is_async_local_call {
                self.mark_expr_needs_send(reference.expr.as_ref());
            }

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
        let method_name = method.to_string();
        let is_mut = self
            .method_registry
            .is_mutating_by_name(&method_name)
            .unwrap_or_else(|| is_mutating_method(method));
        let use_kind = if is_mut {
            UseKind::Write
        } else {
            UseKind::Read
        };

        if self.emit_method_use(receiver, use_kind, &method_name) {
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

    // -----------------------------------------------------------------------
    // Spawn-block handling [S-8 / S-9]
    // -----------------------------------------------------------------------

    /// Name of the marker macro emitted by the spawn preprocessor.
    const SPAWN_MACRO_NAME: &'static str = "__kobo_spawn_block";

    /// Walk a macro statement, detecting `__kobo_spawn_block!` for capture analysis.
    fn walk_macro_stmt(&mut self, mac: &syn::Macro) {
        let is_spawn = mac
            .path
            .get_ident()
            .map(|id| id == Self::SPAWN_MACRO_NAME)
            .unwrap_or(false);

        if !is_spawn {
            self.walk_non_spawn_macro(mac);
            return;
        }

        // Parse the macro body as a Block.
        let Ok(block) = mac.parse_body::<syn::Block>() else {
            return;
        };

        // Enter spawn scope — bindings used inside will be tracked as captured.
        self.spawn_depth += 1;
        let prev_captured = std::mem::take(&mut self.spawn_captured);

        self.walk_block(&block);

        // Collect captured bindings and create a spawn site.
        let captured = std::mem::replace(&mut self.spawn_captured, prev_captured);
        self.spawn_depth -= 1;

        if !captured.is_empty() {
            let span = self.ast.span_from_syn(syn::spanned::Spanned::span(mac));
            self.spawn_sites.push(crate::escape::SpawnSite {
                span,
                captured_bindings: captured,
            });
        }
    }

    fn walk_non_spawn_macro(&mut self, mac: &syn::Macro) {
        self.walk_macro_tokens(mac.tokens.clone());
    }

    fn walk_macro_tokens(&mut self, tokens: TokenStream) {
        let tokens = tokens.into_iter().collect::<Vec<_>>();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Ident(ident) => {
                    if self.emit_ident_use(ident, UseKind::Read)
                        && token_is_dot(tokens.get(index + 1))
                    {
                        self.mark_ident_method_read(ident);
                    }
                }
                TokenTree::Group(group) => self.walk_macro_tokens(group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }

    /// Record a binding use as captured if we are inside a spawn block.
    pub(super) fn record_spawn_capture(&mut self, decl_id: kobo_ir::KirNodeId) {
        if self.spawn_depth > 0 && !self.spawn_captured.contains(&decl_id) {
            self.spawn_captured.push(decl_id);
        }
    }

    // -----------------------------------------------------------------------
    // Attribute collection helpers (G5, G6, Trap 12)
    // -----------------------------------------------------------------------

    /// Collect `#[kobo::relax]` attributes on a function, pushing ranges and errors [G5].
    /// Returns the count of relax attributes found.
    fn collect_relax_attrs_on_fn(&mut self, attrs: &[syn::Attribute], fn_span: KoboSpan) -> usize {
        let mut count = 0usize;
        for attr in attrs {
            match parse_relax_attr(attr) {
                RelaxAttrResult::Valid(attr_span) => {
                    count += 1;
                    if count == 1 {
                        self.relaxed_fn_ranges.push(fn_span);
                    } else {
                        let span = self.ast.span_from_syn(attr_span);
                        self.relax_attr_errors.push(RelaxAttrError {
                            span,
                            message: "duplicate `#[kobo::relax]` attribute".to_owned(),
                            is_error: false,
                        });
                    }
                }
                RelaxAttrResult::HasArguments(attr_span) => {
                    let span = self.ast.span_from_syn(attr_span);
                    self.relax_attr_errors.push(RelaxAttrError {
                        span,
                        message: "`#[kobo::relax]` takes no arguments".to_owned(),
                        is_error: false,
                    });
                }
                RelaxAttrResult::NotRelax => {}
            }
        }
        count
    }

    /// Trap 12: if relax was found on an @strict function, discard relax and warn [BUG-07].
    fn check_relax_strict_conflict(&mut self, relax_count: usize, fn_span: KoboSpan) {
        if relax_count == 0 {
            return;
        }
        let is_strict = self.ast.strict_fns().iter().any(|sf| sf.span == fn_span);
        if is_strict {
            self.relaxed_fn_ranges.retain(|s| *s != fn_span);
            self.relax_attr_errors.push(RelaxAttrError {
                span: fn_span,
                message: "`#[kobo::relax]` has no effect on `@strict` functions".to_owned(),
                is_error: false,
            });
        }
    }

    /// Collect `#[kobo::migrate]` attributes on an item, pushing sites and errors [G6].
    fn collect_migrate_attrs(&mut self, attrs: &[syn::Attribute], target: MigrateTarget) {
        let mut count = 0usize;
        for attr in attrs {
            match parse_migrate_attr(attr) {
                MigrateAttrResult::Valid(attr_span) => {
                    self.record_migrate_site(&mut count, attr_span, target, None);
                }
                MigrateAttrResult::ValidWithReason(reason, attr_span) => {
                    self.record_migrate_site(&mut count, attr_span, target, Some(reason));
                }
                MigrateAttrResult::HasArguments(attr_span) => {
                    let span = self.ast.span_from_syn(attr_span);
                    self.relax_attr_errors.push(RelaxAttrError {
                        span,
                        message: "`#[kobo::migrate]` does not accept parenthesized arguments"
                            .to_owned(),
                        is_error: true,
                    });
                }
                MigrateAttrResult::NotMigrate => {}
            }
        }
    }

    fn record_migrate_site(
        &mut self,
        count: &mut usize,
        attr_span: proc_macro2::Span,
        target: MigrateTarget,
        reason: Option<String>,
    ) {
        *count += 1;
        let span = self.ast.span_from_syn(attr_span);
        if *count == 1 {
            self.migrate_sites.push(MigrateSite {
                span,
                target,
                reason,
            });
        } else {
            self.relax_attr_errors.push(RelaxAttrError {
                span,
                message: "duplicate `#[kobo::migrate]` attribute".to_owned(),
                is_error: false,
            });
        }
    }
}

fn token_is_dot(token: Option<&TokenTree>) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == '.')
}
