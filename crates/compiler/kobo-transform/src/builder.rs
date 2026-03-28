use std::collections::{HashMap, HashSet};

use syn::spanned::Spanned;

use crate::box_reason::{collect_box_reasons, type_box_reason};
use crate::classify::{binding_metadata, BindingMetadata, BindingState, TransformCtx};
use crate::escape::BorrowAlias;
use crate::hint::parse_hint;
use crate::options::TransformOptions;
use kobo_ir::{
    BindingUsage, BoxReason, EscapeKind, KirNode, NodeIdGen, NodeKind, OwnershipHint,
    OwnershipTier, TransformBindingFacts, TransformFacts, UseEvent, UseKind,
};
use kobo_parser::{KoboBinding, KoboFile};

#[derive(Clone, Copy, Debug)]
struct PendingHint {
    hint: OwnershipHint,
    span: kobo_ir::KoboSpan,
}

struct FunctionCtx {
    generic_params: HashSet<String>,
    is_async: bool,
    hint: Option<PendingHint>,
    hint_consumed: bool,
}

pub(crate) struct BuilderOutput {
    pub(crate) nodes: Vec<KirNode>,
    pub(crate) transform_facts: TransformFacts,
    pub(crate) borrow_aliases: Vec<BorrowAlias>,
}

pub(crate) struct TransformFactsBuilder<'a> {
    ast: &'a KoboFile,
    id_gen: &'a mut NodeIdGen,
    ctx: TransformCtx,
    nodes: Vec<KirNode>,
    transform_facts: TransformFacts,
    fact_indices: HashMap<kobo_ir::KirNodeId, usize>,
    borrow_aliases: Vec<BorrowAlias>,
    function_stack: Vec<FunctionCtx>,
    box_reasons: HashMap<String, BoxReason>,
}

pub(crate) fn build_transform_builder<'a>(
    ast: &'a KoboFile,
    id_gen: &'a mut NodeIdGen,
    options: TransformOptions,
) -> TransformFactsBuilder<'a> {
    let mut builder = TransformFactsBuilder::new(ast, id_gen, options);
    for item in &ast.inner.items {
        builder.walk_item(item);
    }
    builder
}

impl<'a> TransformFactsBuilder<'a> {
    pub(crate) fn new(ast: &'a KoboFile, id_gen: &'a mut NodeIdGen, options: TransformOptions) -> Self {
        Self {
            ast,
            id_gen,
            ctx: TransformCtx::new(),
            nodes: Vec::new(),
            transform_facts: TransformFacts::default(),
            fact_indices: HashMap::new(),
            borrow_aliases: Vec::new(),
            function_stack: Vec::new(),
            box_reasons: collect_box_reasons(ast, options.small_struct_clone_threshold_bytes),
        }
    }

    pub(crate) fn finish(self) -> BuilderOutput {
        let mut transform_facts = self.transform_facts;
        let derived = kobo_ir::derive_transform_facts(
            transform_facts
                .bindings
                .iter()
                .map(|binding| binding.usage.clone())
                .collect(),
            Vec::new(),
            std::mem::take(&mut transform_facts.hint_conflicts),
        );
        transform_facts.usages = derived.usages;
        transform_facts.shared_facts = derived.shared_facts;
        transform_facts.hint_conflicts = derived.hint_conflicts;

        BuilderOutput {
            nodes: self.nodes,
            transform_facts,
            borrow_aliases: self.borrow_aliases,
        }
    }

    fn walk_item(&mut self, item: &syn::Item) {
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
            .collect::<HashSet<_>>();
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
            if let Some(alias) = self.borrow_alias_from_initializer(binding_state.decl_id, init_expr)
            {
                self.borrow_aliases.push(alias);
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
                self.walk_assignment_target(assign.left.as_ref());
                self.walk_move_context_expr(assign.right.as_ref(), None);
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
                    self.walk_move_context_expr(expr.as_ref(), Some(EscapeKind::ReturnedFromFunction));
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
        for argument in &call.args {
            let syn::Expr::Reference(reference) = argument else {
                self.walk_move_context_expr(argument, Some(EscapeKind::PassedToOpaqueCall));
                continue;
            };

            if self.emit_ephemeral_borrow(reference.expr.as_ref(), borrow_kind(reference)) {
                continue;
            }

            self.walk_expr(argument);
        }
    }

    fn walk_local_initializer(&mut self, expr: &syn::Expr) {
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

    fn emit_declared_binding(
        &mut self,
        ident: &syn::Ident,
        init: Option<&syn::Expr>,
        hint: Option<PendingHint>,
    ) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span).cloned() else {
            return;
        };

        let binding_state = self.emit_binding(&binding, init, hint);
        if let Some(init_expr) = init {
            self.walk_local_initializer(init_expr);
        }
        self.ctx.define(&binding.ident, binding_state);
    }

    fn emit_pat_binding(
        &mut self,
        pat: &syn::Pat,
        init: Option<&syn::Expr>,
        hint: Option<PendingHint>,
    ) {
        let Some(binding) = self.binding_for_pat(pat).cloned() else {
            return;
        };

        let binding_state = self.emit_binding(&binding, init, hint);
        self.ctx.define(&binding.ident, binding_state);
    }

    fn emit_binding(
        &mut self,
        binding: &KoboBinding,
        init: Option<&syn::Expr>,
        hint: Option<PendingHint>,
    ) -> BindingState {
        let metadata = self.binding_metadata(binding, init);
        let decl_id = self.id_gen.next_kir_id();
        self.nodes.push(build_decl_node(binding, decl_id, metadata));
        self.push_binding_fact(binding, decl_id, metadata, hint);

        BindingState {
            decl_id,
            resource_kind: metadata.resource_kind,
            is_copy_known: metadata.is_copy_known,
            is_generic: metadata.is_generic,
        }
    }

    fn binding_metadata(&self, binding: &KoboBinding, init: Option<&syn::Expr>) -> BindingMetadata {
        let generic_params = self
            .function_stack
            .last()
            .map(|frame| &frame.generic_params)
            .cloned()
            .unwrap_or_default();
        let mut metadata = binding_metadata(binding, init, &self.ctx, &generic_params);
        if metadata.resource_kind.is_none() {
            metadata.resource_kind = detect_boxless_resource(binding, init, &self.ctx);
        }
        metadata
    }

    fn push_binding_fact(
        &mut self,
        binding: &KoboBinding,
        decl_id: kobo_ir::KirNodeId,
        metadata: BindingMetadata,
        hint: Option<PendingHint>,
    ) {
        let box_reason = self.detect_box_reason(binding);
        let shared_facts = kobo_ir::SharedBindingFacts {
            box_reason,
            ..Default::default()
        };
        self.fact_indices
            .insert(decl_id, self.transform_facts.bindings.len());
        self.transform_facts.bindings.push(TransformBindingFacts {
            node: decl_id,
            ast_id: binding.id,
            binding_name: binding.ident.to_string(),
            span: binding.span,
            resource_kind: metadata.resource_kind,
            hint: hint.map(|pending| pending.hint),
            hint_span: hint.map(|pending| pending.span),
            is_copy_known: metadata.is_copy_known,
            is_generic: metadata.is_generic,
            is_async: self
                .function_stack
                .last()
                .is_some_and(|frame| frame.is_async),
            usage: BindingUsage::new(binding.span),
            shared_facts,
            clone_elision: None,
            elision_fallback: None,
        });
    }

    fn detect_box_reason(&self, binding: &KoboBinding) -> Option<BoxReason> {
        binding
            .ty
            .as_ref()
            .and_then(|ty| type_box_reason(ty, &self.box_reasons))
    }

    fn binding_for_pat(&self, pat: &syn::Pat) -> Option<&KoboBinding> {
        let ident = binding_ident(pat)?;
        let span = self.ast.span_from_syn(ident.span());
        self.ast.binding_for_span(span)
    }

    fn take_function_hint(&mut self) -> Option<PendingHint> {
        let frame = self.function_stack.last_mut()?;
        if frame.hint_consumed {
            return None;
        }
        frame.hint_consumed = true;
        frame.hint
    }

    fn borrow_alias_from_initializer(
        &self,
        alias_decl: kobo_ir::KirNodeId,
        expr: &syn::Expr,
    ) -> Option<BorrowAlias> {
        let syn::Expr::Reference(reference) = expr else {
            return None;
        };
        let (binding_state, span) = self.resolved_binding(reference.expr.as_ref())?;
        Some(BorrowAlias {
            alias: alias_decl,
            source: binding_state.decl_id,
            kind: borrow_kind(reference),
            span,
        })
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

    fn emit_move(&mut self, expr: &syn::Expr, escape_kind: Option<EscapeKind>) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Move,
            span,
            binding_state.decl_id,
        ));
        self.record_use_event(binding_state.decl_id, UseEvent::Moved { span });
        if let Some(kind) = escape_kind {
            self.record_use_event(binding_state.decl_id, UseEvent::Escaped { kind, span });
        }
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
            binding_state.decl_id,
        ));
        let event = match use_kind {
            UseKind::Read => UseEvent::ReadOnly { span },
            UseKind::Write => UseEvent::Mutated { span },
        };
        self.record_use_event(binding_state.decl_id, event);
        true
    }

    fn emit_borrow(&mut self, expr: &syn::Expr, borrow_kind: kobo_ir::BorrowKind) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Borrow(borrow_kind),
            span,
            binding_state.decl_id,
        ));
        self.record_use_event(
            binding_state.decl_id,
            UseEvent::Borrowed {
                kind: borrow_kind,
                span,
            },
        );
        true
    }

    fn emit_ephemeral_borrow(
        &mut self,
        expr: &syn::Expr,
        borrow_kind: kobo_ir::BorrowKind,
    ) -> bool {
        let Some((binding_state, span)) = self.resolved_binding(expr) else {
            return false;
        };

        let use_kind = match borrow_kind {
            kobo_ir::BorrowKind::Immutable => UseKind::Read,
            kobo_ir::BorrowKind::Mutable => UseKind::Write,
        };
        self.nodes.push(build_binding_event_node(
            self.id_gen.next_kir_id(),
            NodeKind::Use(use_kind),
            span,
            binding_state.decl_id,
        ));
        self.record_use_event(
            binding_state.decl_id,
            UseEvent::Borrowed {
                kind: borrow_kind,
                span,
            },
        );
        true
    }

    fn record_use_event(&mut self, decl_id: kobo_ir::KirNodeId, event: UseEvent) {
        let Some(index) = self.fact_indices.get(&decl_id).copied() else {
            return;
        };
        self.transform_facts.bindings[index].usage.push_use(event);
    }

    fn resolved_binding(&self, expr: &syn::Expr) -> Option<(BindingState, kobo_ir::KoboSpan)> {
        let ident = expr_ident(expr)?;
        let binding_state = self.ctx.lookup(ident)?;
        let span = self.ast.span_from_syn(ident.span());
        Some((binding_state, span))
    }
}

fn detect_boxless_resource(
    binding: &KoboBinding,
    init: Option<&syn::Expr>,
    ctx: &TransformCtx,
) -> Option<kobo_ir::ResourceKind> {
    crate::classify::detect_resource_kind(binding.ty.as_ref(), init, ctx)
}

fn build_decl_node(
    binding: &KoboBinding,
    id: kobo_ir::KirNodeId,
    metadata: BindingMetadata,
) -> KirNode {
    KirNode {
        id,
        kind: NodeKind::Decl,
        ast_id: Some(binding.id),
        ownership: OwnershipTier::Undecided,
        resource_kind: metadata.resource_kind,
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
    decl_id: kobo_ir::KirNodeId,
) -> KirNode {
    KirNode {
        id,
        kind,
        ast_id: None,
        ownership: OwnershipTier::Undecided,
        resource_kind: None,
        cfg_block: None,
        span,
        decl_id: Some(decl_id),
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

fn borrow_kind(reference: &syn::ExprReference) -> kobo_ir::BorrowKind {
    if reference.mutability.is_some() {
        kobo_ir::BorrowKind::Mutable
    } else {
        kobo_ir::BorrowKind::Immutable
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

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, OwnershipTier, TransformFacts, UseEvent};
    use kobo_parser::parse_file;

    use crate::finalize::finalize_transform;
    use crate::options::TransformOptions;
    use crate::transform::build_kir;

    use super::build_transform_builder;

    type Facts = TransformFacts;

    fn transform_facts_for(source: &str) -> Facts {
        let mut id_gen = kobo_ir::NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        finalize_transform(build_transform_builder(&ast, &mut id_gen, TransformOptions::default()))
            .transform_facts
    }

    fn tier_for_binding(source: &str, name: &str, occurrence: usize) -> OwnershipTier {
        let mut id_gen = kobo_ir::NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let binding = kir
            .transform_facts()
            .iter_bindings()
            .filter(|binding| binding.binding_name == name)
            .nth(occurrence)
            .expect("binding should exist");
        kir.tier_decision(binding.node)
            .expect("decision should exist")
            .tier
    }

    #[test]
    fn escape_analysis_is_deterministic_for_same_source() {
        let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    let z = x;
    use_ref(y);
    println!("{}", z.len());
}
"#;

        let first = transform_facts_for(source);
        let second = transform_facts_for(source);

        assert_eq!(first.usages, second.usages);
    }

    #[test]
    fn shadowed_bindings_stay_separate_in_builder_output() {
        let source = r#"
fn main() {
    let x = String::from("outer");
    {
        let mut x: Vec<String> = Vec::new();
        x.push(String::from("inner"));
        println!("{}", x.len());
    }
    println!("{}", x.len());
}
"#;

        let facts = transform_facts_for(source);
        let xs = facts
            .iter_bindings()
            .filter(|binding| binding.binding_name == "x")
            .collect::<Vec<_>>();

        assert_eq!(xs.len(), 2);
        assert_ne!(xs[0].node, xs[1].node);
        assert_ne!(xs[0].usage.declaration, xs[1].usage.declaration);
    }

    #[test]
    fn conditional_mutation_marks_mutable_usage_inside_if_branch() {
        let source = r#"
fn mutate<T>(_value: &mut T) {}

fn main() {
    let x = String::from("kobo");
    if true {
        mutate(&mut x);
    }
    println!("{}", x.len());
}
"#;

        let facts = transform_facts_for(source);
        let binding = facts
            .iter_bindings()
            .find(|binding| binding.binding_name == "x")
            .expect("x binding should exist");

        assert!(binding
            .usage
            .uses
            .iter()
            .any(|event| {
                matches!(
                    event,
                    UseEvent::Mutated { .. }
                        | UseEvent::Borrowed {
                            kind: kobo_ir::BorrowKind::Mutable,
                            ..
                        }
                )
            }));
        assert!(binding.shared_facts.needs_mutable_wrapper);
    }

    #[test]
    fn live_borrow_at_move_sets_shared_fact() {
        let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    let z = x;
    use_ref(y);
    println!("{}", z.len());
}
"#;

        let facts = transform_facts_for(source);
        let binding = facts
            .iter_bindings()
            .find(|binding| binding.binding_name == "x")
            .expect("x binding should exist");

        assert!(binding.shared_facts.live_borrow_at_move);
        assert!(binding.shared_facts.needs_sharing);
    }

    #[test]
    fn local_borrow_only_stays_plain_owned() {
        let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    use_ref(y);
}
"#;

        assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::PlainOwned);
    }

    #[test]
    fn read_then_mutate_escalates_to_rc_refcell() {
        let source = r#"
fn use_read(value: &String) {
    println!("{}", value.len());
}

fn use_mut(value: &mut String) {
    value.push('!');
}

fn main() {
    let mut x = String::from("hello");
    use_read(&x);
    use_mut(&mut x);
}
"#;

        assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::RcMutShared);
    }

    #[test]
    fn structurally_identical_generics_choose_same_tier() {
        let source = r#"
fn use_mut<T>(_value: &mut T) {}

fn first<T>(mut value: T) {
    use_mut(&mut value);
}

fn second<U>(mut value: U) {
    use_mut(&mut value);
}
"#;

        assert_eq!(
            tier_for_binding(source, "value", 0),
            tier_for_binding(source, "value", 1),
        );
    }
}
