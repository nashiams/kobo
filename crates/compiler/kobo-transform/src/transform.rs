use syn::spanned::Spanned;

use crate::cfg::build_cfg;
use crate::classify::{classify_binding, BindingClassification, BindingState, TransformCtx};
use kobo_ir::{BorrowKind, Kir, KirNode, NodeIdGen, NodeKind, OwnershipTier, UseKind};
use kobo_parser::{KoboBinding, KoboFile};

/// Transforms a parsed Kobo file into the frozen KIR.
///
/// Walks the AST in source order, classifies each binding, and records KIR
/// nodes for declarations, use sites, moves, borrows, and lexical scope
/// boundaries. The CFG stub is built unconditionally so the pipeline hook
/// exists for v0.7.
pub fn build_kir(ast: &KoboFile, id_gen: &mut NodeIdGen) -> Kir {
    let mut builder = KirBuilder::new(ast, id_gen);
    for item in &ast.inner.items {
        builder.walk_item(item);
    }

    let kir = Kir::from_nodes(builder.finish());
    let _cfg = build_cfg(&kir);
    kir
}

struct KirBuilder<'a> {
    ast: &'a KoboFile,
    id_gen: &'a mut NodeIdGen,
    ctx: TransformCtx,
    nodes: Vec<KirNode>,
}

impl<'a> KirBuilder<'a> {
    fn new(ast: &'a KoboFile, id_gen: &'a mut NodeIdGen) -> Self {
        Self {
            ast,
            id_gen,
            ctx: TransformCtx::new(),
            nodes: Vec::new(),
        }
    }

    fn finish(self) -> Vec<KirNode> {
        self.nodes
    }

    fn walk_item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Fn(function) => self.walk_function(function),
            syn::Item::Const(item_const) => {
                self.emit_declared_binding(&item_const.ident, Some(&item_const.expr));
            }
            syn::Item::Static(item_static) => {
                self.emit_declared_binding(&item_static.ident, Some(&item_static.expr));
            }
            _ => {}
        }
    }

    fn walk_function(&mut self, function: &syn::ItemFn) {
        self.enter_scope(self.ast.span_from_syn(function.span()));
        self.ctx.push_scope();

        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                self.emit_pat_binding(&argument.pat, None);
            }
        }

        self.walk_block_statements(&function.block.stmts);

        self.ctx.pop_scope();
        self.exit_scope(self.ast.span_from_syn(function.span()));
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

        let binding_state = self.emit_binding(&binding, init);
        if let Some(init_expr) = init {
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
                self.walk_assignment_target(assign.left.as_ref());
                self.walk_move_context_expr(assign.right.as_ref());
            }
            syn::Expr::Binary(binary) => {
                self.walk_expr(binary.left.as_ref());
                self.walk_expr(binary.right.as_ref());
            }
            syn::Expr::Block(block) => self.walk_block(&block.block),
            syn::Expr::Call(call) => {
                self.walk_expr(call.func.as_ref());
                for argument in &call.args {
                    self.walk_move_context_expr(argument);
                }
            }
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
                    self.walk_move_context_expr(expr.as_ref());
                }
            }
            syn::Expr::Struct(expr_struct) => {
                for field in &expr_struct.fields {
                    self.walk_expr(&field.expr);
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

    fn walk_local_initializer(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Reference(reference) => self.walk_reference_expr(reference),
            _ => self.walk_move_context_expr(expr),
        }
    }

    fn walk_move_context_expr(&mut self, expr: &syn::Expr) {
        if self.emit_move_or_use(expr) {
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

    fn emit_declared_binding(&mut self, ident: &syn::Ident, init: Option<&syn::Expr>) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span).cloned() else {
            return;
        };

        let binding_state = self.emit_binding(&binding, init);
        if let Some(init_expr) = init {
            self.walk_local_initializer(init_expr);
        }
        self.ctx.define(&binding.ident, binding_state);
    }

    fn emit_pat_binding(&mut self, pat: &syn::Pat, init: Option<&syn::Expr>) {
        let Some(binding) = self.binding_for_pat(pat).cloned() else {
            return;
        };

        let binding_state = self.emit_binding(&binding, init);
        self.ctx.define(&binding.ident, binding_state);
    }

    fn emit_binding(&mut self, binding: &KoboBinding, init: Option<&syn::Expr>) -> BindingState {
        let classification = classify_binding(binding, init, &self.ctx);
        let decl_id = self.id_gen.next_kir_id();
        self.nodes
            .push(build_decl_node(binding, classification, decl_id));

        BindingState {
            decl_id,
            ownership: classification.ownership,
            resource_kind: classification.resource_kind,
        }
    }

    fn binding_for_pat(&self, pat: &syn::Pat) -> Option<&KoboBinding> {
        let ident = binding_ident(pat)?;
        let span = self.ast.span_from_syn(ident.span());
        self.ast.binding_for_span(span)
    }

    fn enter_scope(&mut self, span: kobo_ir::KoboSpan) {
        self.nodes.push(build_scope_node(
            self.id_gen.next_kir_id(),
            NodeKind::ScopeStart,
            span,
        ));
    }

    fn exit_scope(&mut self, span: kobo_ir::KoboSpan) {
        self.nodes.push(build_scope_node(
            self.id_gen.next_kir_id(),
            NodeKind::ScopeEnd,
            span,
        ));
    }

    fn emit_move_or_use(&mut self, expr: &syn::Expr) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        let node_kind = if binding_state.ownership == OwnershipTier::PlainOwned {
            NodeKind::Use(UseKind::Read)
        } else {
            NodeKind::Move
        };
        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            node_kind,
            span,
            binding_state,
        ));
        true
    }

    fn emit_use(&mut self, expr: &syn::Expr, use_kind: UseKind) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Use(use_kind),
            span,
            binding_state,
        ));
        true
    }

    fn emit_borrow(&mut self, expr: &syn::Expr, borrow_kind: BorrowKind) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Borrow(borrow_kind),
            span,
            binding_state,
        ));
        true
    }

    fn resolved_binding(&self, expr: &syn::Expr) -> Option<(BindingState, kobo_ir::KoboSpan)> {
        let ident = expr_ident(expr)?;
        let binding_state = self.ctx.lookup(ident)?;
        let span = self.ast.span_from_syn(ident.span());
        Some((binding_state, span))
    }
}

fn build_decl_node(
    binding: &KoboBinding,
    classification: BindingClassification,
    id: kobo_ir::KirNodeId,
) -> KirNode {
    KirNode {
        id,
        kind: NodeKind::Decl,
        ast_id: Some(binding.id),
        ownership: classification.ownership,
        resource_kind: classification.resource_kind,
        cfg_block: None,
        span: binding.span,
        decl_id: None,
    }
}

fn build_scope_node(id: kobo_ir::KirNodeId, kind: NodeKind, span: kobo_ir::KoboSpan) -> KirNode {
    KirNode {
        id,
        kind,
        ast_id: None,
        ownership: OwnershipTier::PlainOwned,
        resource_kind: None,
        cfg_block: None,
        span,
        decl_id: None,
    }
}

fn build_binding_event_node(
    id: kobo_ir::KirNodeId,
    kind: NodeKind,
    span: kobo_ir::KoboSpan,
    binding_state: BindingState,
) -> KirNode {
    KirNode {
        id,
        kind,
        ast_id: None,
        ownership: binding_state.ownership,
        resource_kind: binding_state.resource_kind,
        cfg_block: None,
        span,
        decl_id: Some(binding_state.decl_id),
    }
}

fn binding_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}

fn expr_ident(expr: &syn::Expr) -> Option<&syn::Ident> {
    match expr {
        syn::Expr::Path(path) => {
            if path.qself.is_some() || path.path.segments.len() != 1 {
                return None;
            }

            Some(&path.path.segments.first()?.ident)
        }
        syn::Expr::Paren(paren) => expr_ident(paren.expr.as_ref()),
        syn::Expr::Group(group) => expr_ident(group.expr.as_ref()),
        _ => None,
    }
}

fn borrow_kind(reference: &syn::ExprReference) -> BorrowKind {
    if reference.mutability.is_some() {
        BorrowKind::Mutable
    } else {
        BorrowKind::Immutable
    }
}

fn is_mutating_method(method: &syn::Ident) -> bool {
    matches!(
        method.to_string().as_str(),
        "append"
            | "clear"
            | "extend"
            | "insert"
            | "insert_str"
            | "pop"
            | "push"
            | "push_str"
            | "remove"
            | "replace"
            | "retain"
            | "reverse"
            | "sort"
            | "sort_by"
            | "swap"
            | "truncate"
    )
}
