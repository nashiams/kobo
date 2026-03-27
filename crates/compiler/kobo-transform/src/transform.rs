use crate::cfg::build_cfg;
use crate::classify::{classify_binding, BindingClassification, BindingState, TransformCtx};
use kobo_ir::{Kir, KirNode, NodeIdGen};
use kobo_parser::{KoboBinding, KoboFile};

/// Transforms a parsed Kobo file into the frozen KIR.
///
/// Walks the AST in source order, classifies each binding, and records a KIR
/// node for every ownership-relevant binding. The CFG stub is built
/// unconditionally so the pipeline hook exists for v0.7.
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
        self.ctx.push_scope();

        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                self.emit_pat_binding(&argument.pat, None);
            }
        }

        self.walk_block(&function.block);
        self.ctx.pop_scope();
    }

    fn walk_block(&mut self, block: &syn::Block) {
        self.ctx.push_scope();

        for stmt in &block.stmts {
            self.walk_stmt(stmt);
        }

        self.ctx.pop_scope();
    }

    fn walk_stmt(&mut self, stmt: &syn::Stmt) {
        match stmt {
            syn::Stmt::Local(local) => {
                let init = local.init.as_ref().map(|init| init.expr.as_ref());
                self.emit_pat_binding(&local.pat, init);

                if let Some(init_expr) = init {
                    self.walk_expr(init_expr);
                }
            }
            syn::Stmt::Item(item) => self.walk_item(item),
            syn::Stmt::Expr(expr, _) => self.walk_expr(expr),
            syn::Stmt::Macro(_) => {}
        }
    }

    fn walk_expr(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Block(block) => self.walk_block(&block.block),
            syn::Expr::If(expr_if) => {
                self.walk_expr(&expr_if.cond);
                self.walk_block(&expr_if.then_branch);

                if let Some((_, else_branch)) = &expr_if.else_branch {
                    self.walk_expr(else_branch);
                }
            }
            syn::Expr::ForLoop(for_loop) => self.walk_block(&for_loop.body),
            syn::Expr::While(expr_while) => {
                self.walk_expr(&expr_while.cond);
                self.walk_block(&expr_while.body);
            }
            syn::Expr::Loop(expr_loop) => self.walk_block(&expr_loop.body),
            syn::Expr::Match(expr_match) => {
                self.walk_expr(&expr_match.expr);
                for arm in &expr_match.arms {
                    self.walk_expr(&arm.body);
                    if let Some((_, guard)) = &arm.guard {
                        self.walk_expr(guard);
                    }
                }
            }
            syn::Expr::Closure(closure) => self.walk_expr(&closure.body),
            syn::Expr::Paren(paren) => self.walk_expr(&paren.expr),
            syn::Expr::Group(group) => self.walk_expr(&group.expr),
            syn::Expr::Assign(assign) => {
                self.walk_expr(&assign.left);
                self.walk_expr(&assign.right);
            }
            syn::Expr::Call(call) => {
                self.walk_expr(&call.func);
                for argument in &call.args {
                    self.walk_expr(argument);
                }
            }
            syn::Expr::MethodCall(method_call) => {
                self.walk_expr(&method_call.receiver);
                for argument in &method_call.args {
                    self.walk_expr(argument);
                }
            }
            syn::Expr::Array(array) => {
                for element in &array.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Return(return_expr) => {
                if let Some(expr) = &return_expr.expr {
                    self.walk_expr(expr);
                }
            }
            _ => {}
        }
    }

    fn emit_declared_binding(&mut self, ident: &syn::Ident, init: Option<&syn::Expr>) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span) else {
            return;
        };

        self.emit_binding(binding, init);
    }

    fn emit_pat_binding(&mut self, pat: &syn::Pat, init: Option<&syn::Expr>) {
        let Some(ident) = binding_ident(pat) else {
            return;
        };

        self.emit_declared_binding(ident, init);
    }

    fn emit_binding(&mut self, binding: &KoboBinding, init: Option<&syn::Expr>) {
        let classification = classify_binding(binding, init, &self.ctx);
        self.nodes.push(build_kir_node(
            binding,
            classification,
            self.id_gen.next_kir_id(),
        ));
        self.ctx.define(
            &binding.ident,
            BindingState {
                ownership: classification.ownership,
                resource_kind: classification.resource_kind,
            },
        );
    }
}

fn build_kir_node(
    binding: &KoboBinding,
    classification: BindingClassification,
    id: kobo_ir::KirNodeId,
) -> KirNode {
    KirNode {
        id,
        ast_id: binding.id,
        ownership: classification.ownership,
        resource_kind: classification.resource_kind,
        cfg_block: None,
        span: binding.span,
    }
}

fn binding_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}
