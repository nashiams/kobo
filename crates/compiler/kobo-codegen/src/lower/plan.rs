use std::collections::{HashMap, HashSet};

use kobo_ir::{
    KirNodeId, KoboAstNodeId, KoboSpan, NodeKind, OwnershipTier, SatisfactionCheck, SolutionMap,
    TierReason,
};
use kobo_parser::{KoboBinding, KoboFile};
use syn::spanned::Spanned;

use super::binding::binding_for_pat;
use super::support::support_items;

#[derive(Clone, Debug)]
pub struct LoweringSite {
    pub node: KirNodeId,
    pub binding_name: String,
    pub ownership_tier: OwnershipTier,
    pub kobo_span: KoboSpan,
    pub kobo_line: usize,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct AnnotationNote {
    pub node: KirNodeId,
    pub binding_name: String,
    pub kobo_line: usize,
    pub reason: String,
}

impl LoweringSite {
    pub(crate) fn display_name(&self) -> &str {
        &self.binding_name
    }

    #[cfg(test)]
    pub(crate) fn new(
        node: KirNodeId,
        binding_name: impl Into<String>,
        ownership_tier: OwnershipTier,
        kobo_span: KoboSpan,
        kobo_line: usize,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            node,
            binding_name: binding_name.into(),
            ownership_tier,
            kobo_span,
            kobo_line,
            reason: reason.into(),
        }
    }
}

impl AnnotationNote {
    pub(crate) fn display_name(&self) -> &str {
        &self.binding_name
    }

    #[cfg(test)]
    pub(crate) fn new(
        node: KirNodeId,
        binding_name: impl Into<String>,
        kobo_line: usize,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            node,
            binding_name: binding_name.into(),
            kobo_line,
            reason: reason.into(),
        }
    }
}

pub(crate) struct LoweringPlan {
    nodes_by_ast: HashMap<KoboAstNodeId, KirNodeId>,
    lowering_tiers_by_ast: HashMap<KoboAstNodeId, OwnershipTier>,
    function_param_tiers: HashMap<String, Vec<OwnershipTier>>,
    annotation_sites: Vec<LoweringSite>,
    annotation_notes: Vec<AnnotationNote>,
    plain_clone_aliases: HashSet<KirNodeId>,
    needs_rc: bool,
    needs_refcell: bool,
    needs_arc: bool,
    needs_scoped_handle: bool,
    support_item_count: usize,
}

impl LoweringPlan {
    pub(crate) fn from_kir(ast: &KoboFile, kir: &kobo_ir::Kir, solution: &SolutionMap) -> Self {
        let mut nodes_by_ast = HashMap::new();
        let mut lowering_tiers_by_ast = HashMap::new();
        let mut annotation_tiers_by_ast = HashMap::new();
        let mut needs_rc = false;
        let mut needs_refcell = false;
        let mut needs_arc = false;
        let mut needs_scoped_handle = false;

        for node in kir.iter_decl_nodes() {
            debug_assert_eq!(node.kind, NodeKind::Decl);
            let resolved_tier = solution.resolve(node.id, node.ownership);
            let Some(ast_id) = node.ast_id else {
                unreachable!("invariant: declaration nodes must carry KoboAstNodeId");
            };
            let lowering_tier = effective_lowering_tier(kir, node.id, resolved_tier);
            nodes_by_ast.insert(ast_id, node.id);
            lowering_tiers_by_ast.insert(ast_id, lowering_tier);
            annotation_tiers_by_ast.insert(ast_id, resolved_tier);

            needs_rc |= matches!(
                lowering_tier,
                OwnershipTier::RcShared | OwnershipTier::RcMutShared
            );
            needs_refcell |= lowering_tier == OwnershipTier::RcMutShared;
            needs_arc |= matches!(
                lowering_tier,
                OwnershipTier::ArcShared | OwnershipTier::ArcMutShared
            );
            needs_scoped_handle |= lowering_tier == OwnershipTier::Scoped;
        }

        let function_param_tiers = build_function_param_tiers(ast, &lowering_tiers_by_ast);
        let annotation_sites = build_annotation_sites(ast, kir, &annotation_tiers_by_ast);
        let annotation_notes = build_annotation_notes(ast, kir);
        let plain_clone_aliases = kir
            .transform_facts()
            .iter_bindings()
            .filter(|binding| binding.plain_clone_alias)
            .map(|binding| binding.node)
            .collect();
        let support_item_count =
            support_items(needs_rc, needs_refcell, needs_arc, needs_scoped_handle).len();

        Self {
            nodes_by_ast,
            lowering_tiers_by_ast,
            function_param_tiers,
            annotation_sites,
            annotation_notes,
            plain_clone_aliases,
            needs_rc,
            needs_refcell,
            needs_arc,
            needs_scoped_handle,
            support_item_count,
        }
    }

    pub(crate) fn tier_for_binding(&self, binding: &KoboBinding) -> OwnershipTier {
        *self
            .lowering_tiers_by_ast
            .get(&binding.id)
            .unwrap_or(&OwnershipTier::PlainOwned)
    }

    pub(crate) fn called_function_param_tiers(&self, expr: &syn::Expr) -> Option<&[OwnershipTier]> {
        let syn::Expr::Path(path) = expr else {
            return None;
        };
        if path.path.segments.len() != 1 {
            return None;
        }

        let function_name = path.path.segments.first()?.ident.to_string();
        self.function_param_tiers
            .get(&function_name)
            .map(Vec::as_slice)
    }

    pub(crate) fn insert_support_items(&self, file: &mut syn::File) {
        let mut prepended_items = support_items(
            self.needs_rc,
            self.needs_refcell,
            self.needs_arc,
            self.needs_scoped_handle,
        );
        if prepended_items.is_empty() {
            return;
        }

        prepended_items.append(&mut file.items);
        file.items = prepended_items;
    }

    pub(crate) fn annotation_sites(&self) -> &[LoweringSite] {
        &self.annotation_sites
    }

    pub(crate) fn annotation_notes(&self) -> &[AnnotationNote] {
        &self.annotation_notes
    }

    pub(crate) fn node_for_binding(&self, binding: &KoboBinding) -> Option<KirNodeId> {
        self.nodes_by_ast.get(&binding.id).copied()
    }

    pub(crate) fn binding_uses_plain_clone_alias(&self, binding: &KoboBinding) -> bool {
        self.node_for_binding(binding)
            .is_some_and(|node| self.plain_clone_aliases.contains(&node))
    }

    pub(crate) fn support_item_count(&self) -> usize {
        self.support_item_count
    }
}

fn build_function_param_tiers(
    ast: &KoboFile,
    tiers_by_ast: &HashMap<KoboAstNodeId, OwnershipTier>,
) -> HashMap<String, Vec<OwnershipTier>> {
    let mut function_param_tiers = HashMap::new();

    for item in &ast.inner.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };

        let tiers = function
            .sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                syn::FnArg::Typed(argument) => {
                    let binding = binding_for_pat(ast, &argument.pat)?;
                    Some(
                        *tiers_by_ast
                            .get(&binding.id)
                            .unwrap_or(&OwnershipTier::PlainOwned),
                    )
                }
                syn::FnArg::Receiver(_) => None,
            })
            .collect();

        function_param_tiers.insert(function.sig.ident.to_string(), tiers);
    }

    function_param_tiers
}

fn build_annotation_sites(
    ast: &KoboFile,
    kir: &kobo_ir::Kir,
    tiers_by_ast: &HashMap<KoboAstNodeId, OwnershipTier>,
) -> Vec<LoweringSite> {
    let mut sites = Vec::new();

    for binding in ast.iter_bindings() {
        let Some(node_id) = kir.kir_for_ast(binding.id) else {
            continue;
        };
        let Some(decision) = kir.tier_decision(node_id) else {
            continue;
        };
        if !decision.annotate {
            continue;
        }

        let ownership_tier = *tiers_by_ast
            .get(&binding.id)
            .unwrap_or(&OwnershipTier::PlainOwned);
        let (kobo_line, _) = ast.line_col(binding.span);
        sites.push(LoweringSite {
            node: node_id,
            binding_name: binding.ident.to_string(),
            ownership_tier,
            kobo_span: binding.span,
            kobo_line,
            reason: decision.annotation_text(),
        });
    }

    sites
}

fn effective_lowering_tier(
    kir: &kobo_ir::Kir,
    node_id: KirNodeId,
    resolved_tier: OwnershipTier,
) -> OwnershipTier {
    if resolved_tier == OwnershipTier::RcShared && is_return_escape_deferred(kir, node_id) {
        OwnershipTier::PlainOwned
    } else {
        resolved_tier
    }
}

fn is_return_escape_deferred(kir: &kobo_ir::Kir, node_id: KirNodeId) -> bool {
    matches!(
        kir.tier_decision(node_id).map(|decision| &decision.reason),
        Some(TierReason::ValidationEscalation(
            SatisfactionCheck::ReturnEscapeBoxDeferred
        ))
    )
}

fn build_annotation_notes(ast: &KoboFile, kir: &kobo_ir::Kir) -> Vec<AnnotationNote> {
    let mut notes = Vec::new();
    let conditional_body_spans = collect_conditional_body_spans(ast);

    for binding in kir.transform_facts().iter_bindings() {
        let Some(ast_binding) = ast.binding_for_id(binding.ast_id) else {
            continue;
        };
        let (kobo_line, _) = ast.line_col(ast_binding.span);
        if let Some(fallback_reason) = binding.elision_fallback {
            notes.push(AnnotationNote {
                node: binding.node,
                binding_name: ast_binding.ident.to_string(),
                kobo_line,
                reason: kobo_ir::TierReason::CloneElisionFallback(fallback_reason)
                    .annotation_text(OwnershipTier::PlainOwned),
            });
        }
        if let Some(skip_reason) = binding.elision_skip_reason {
            notes.push(AnnotationNote {
                node: binding.node,
                binding_name: ast_binding.ident.to_string(),
                kobo_line,
                reason: skip_reason.annotation_text().to_owned(),
            });
        }
        if has_conditional_mutation_floor(binding, kir, &conditional_body_spans) {
            notes.push(AnnotationNote {
                node: binding.node,
                binding_name: ast_binding.ident.to_string(),
                kobo_line,
                reason: "conditional mutation path".to_owned(),
            });
        }
    }

    notes
}

fn has_conditional_mutation_floor(
    binding: &kobo_ir::TransformBindingFacts,
    kir: &kobo_ir::Kir,
    conditional_body_spans: &[KoboSpan],
) -> bool {
    let Some(decision) = kir.tier_decision(binding.node) else {
        return false;
    };
    if decision.tier != OwnershipTier::RcMutShared {
        return false;
    }

    let mutable_spans = binding
        .usage
        .uses
        .iter()
        .filter_map(|event| match event {
            kobo_ir::UseEvent::Mutated { span }
            | kobo_ir::UseEvent::Borrowed {
                kind: kobo_ir::BorrowKind::Mutable,
                span,
            } => Some(*span),
            _ => None,
        })
        .collect::<Vec<_>>();
    if mutable_spans.is_empty() {
        return false;
    }

    mutable_spans
        .iter()
        .all(|span| span_is_within_any(*span, conditional_body_spans))
}

fn span_is_within_any(span: KoboSpan, containers: &[KoboSpan]) -> bool {
    containers.iter().any(|container| {
        container.file_id == span.file_id
            && container.start <= span.start
            && span.end <= container.end
    })
}

fn collect_conditional_body_spans(ast: &KoboFile) -> Vec<KoboSpan> {
    let mut spans = Vec::new();

    for item in &ast.inner.items {
        collect_conditional_spans_from_item(item, ast, &mut spans);
    }

    spans
}

fn collect_conditional_spans_from_item(
    item: &syn::Item,
    ast: &KoboFile,
    spans: &mut Vec<KoboSpan>,
) {
    match item {
        syn::Item::Fn(function) => {
            collect_conditional_spans_from_block(&function.block, ast, spans)
        }
        syn::Item::Const(item_const) => {
            collect_conditional_spans_from_expr(item_const.expr.as_ref(), ast, spans);
        }
        syn::Item::Static(item_static) => {
            collect_conditional_spans_from_expr(item_static.expr.as_ref(), ast, spans);
        }
        _ => {}
    }
}

fn collect_conditional_spans_from_block(
    block: &syn::Block,
    ast: &KoboFile,
    spans: &mut Vec<KoboSpan>,
) {
    for statement in &block.stmts {
        collect_conditional_spans_from_stmt(statement, ast, spans);
    }
}

fn collect_conditional_spans_from_stmt(
    statement: &syn::Stmt,
    ast: &KoboFile,
    spans: &mut Vec<KoboSpan>,
) {
    match statement {
        syn::Stmt::Local(local) => {
            if let Some(init) = &local.init {
                collect_conditional_spans_from_expr(init.expr.as_ref(), ast, spans);
            }
        }
        syn::Stmt::Item(item) => collect_conditional_spans_from_item(item, ast, spans),
        syn::Stmt::Expr(expr, _) => collect_conditional_spans_from_expr(expr, ast, spans),
        syn::Stmt::Macro(_) => {}
    }
}

fn collect_conditional_spans_from_expr(
    expr: &syn::Expr,
    ast: &KoboFile,
    spans: &mut Vec<KoboSpan>,
) {
    match expr {
        syn::Expr::Array(array) => {
            for element in &array.elems {
                collect_conditional_spans_from_expr(element, ast, spans);
            }
        }
        syn::Expr::Assign(assign) => {
            collect_conditional_spans_from_expr(assign.left.as_ref(), ast, spans);
            collect_conditional_spans_from_expr(assign.right.as_ref(), ast, spans);
        }
        syn::Expr::Binary(binary) => {
            collect_conditional_spans_from_expr(binary.left.as_ref(), ast, spans);
            collect_conditional_spans_from_expr(binary.right.as_ref(), ast, spans);
        }
        syn::Expr::Block(block) => collect_conditional_spans_from_block(&block.block, ast, spans),
        syn::Expr::Call(call) => {
            collect_conditional_spans_from_expr(call.func.as_ref(), ast, spans);
            for argument in &call.args {
                collect_conditional_spans_from_expr(argument, ast, spans);
            }
        }
        syn::Expr::Cast(cast) => {
            collect_conditional_spans_from_expr(cast.expr.as_ref(), ast, spans);
        }
        syn::Expr::Closure(closure) => {
            collect_conditional_spans_from_expr(closure.body.as_ref(), ast, spans);
        }
        syn::Expr::Field(field) => {
            collect_conditional_spans_from_expr(field.base.as_ref(), ast, spans);
        }
        syn::Expr::ForLoop(for_loop) => {
            collect_conditional_spans_from_expr(for_loop.expr.as_ref(), ast, spans);
            spans.push(ast.span_from_syn(for_loop.body.span()));
            collect_conditional_spans_from_block(&for_loop.body, ast, spans);
        }
        syn::Expr::Group(group) => {
            collect_conditional_spans_from_expr(group.expr.as_ref(), ast, spans);
        }
        syn::Expr::If(expr_if) => {
            collect_conditional_spans_from_expr(expr_if.cond.as_ref(), ast, spans);
            spans.push(ast.span_from_syn(expr_if.then_branch.span()));
            collect_conditional_spans_from_block(&expr_if.then_branch, ast, spans);
            if let Some((_, else_branch)) = &expr_if.else_branch {
                spans.push(ast.span_from_syn(else_branch.span()));
                collect_conditional_spans_from_expr(else_branch.as_ref(), ast, spans);
            }
        }
        syn::Expr::Index(index) => {
            collect_conditional_spans_from_expr(index.expr.as_ref(), ast, spans);
            collect_conditional_spans_from_expr(index.index.as_ref(), ast, spans);
        }
        syn::Expr::Loop(expr_loop) => {
            spans.push(ast.span_from_syn(expr_loop.body.span()));
            collect_conditional_spans_from_block(&expr_loop.body, ast, spans);
        }
        syn::Expr::Match(expr_match) => {
            collect_conditional_spans_from_expr(expr_match.expr.as_ref(), ast, spans);
            for arm in &expr_match.arms {
                if let Some((_, guard)) = &arm.guard {
                    collect_conditional_spans_from_expr(guard.as_ref(), ast, spans);
                }
                spans.push(ast.span_from_syn(arm.body.span()));
                collect_conditional_spans_from_expr(arm.body.as_ref(), ast, spans);
            }
        }
        syn::Expr::MethodCall(method_call) => {
            collect_conditional_spans_from_expr(method_call.receiver.as_ref(), ast, spans);
            for argument in &method_call.args {
                collect_conditional_spans_from_expr(argument, ast, spans);
            }
        }
        syn::Expr::Paren(paren) => {
            collect_conditional_spans_from_expr(paren.expr.as_ref(), ast, spans);
        }
        syn::Expr::Reference(reference) => {
            collect_conditional_spans_from_expr(reference.expr.as_ref(), ast, spans);
        }
        syn::Expr::Repeat(repeat) => {
            collect_conditional_spans_from_expr(repeat.expr.as_ref(), ast, spans);
            collect_conditional_spans_from_expr(repeat.len.as_ref(), ast, spans);
        }
        syn::Expr::Return(return_expr) => {
            if let Some(inner) = &return_expr.expr {
                collect_conditional_spans_from_expr(inner.as_ref(), ast, spans);
            }
        }
        syn::Expr::Struct(expr_struct) => {
            for field in &expr_struct.fields {
                collect_conditional_spans_from_expr(&field.expr, ast, spans);
            }
            if let Some(rest) = &expr_struct.rest {
                collect_conditional_spans_from_expr(rest.as_ref(), ast, spans);
            }
        }
        syn::Expr::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_conditional_spans_from_expr(element, ast, spans);
            }
        }
        syn::Expr::Unary(unary) => {
            collect_conditional_spans_from_expr(unary.expr.as_ref(), ast, spans);
        }
        syn::Expr::While(expr_while) => {
            collect_conditional_spans_from_expr(expr_while.cond.as_ref(), ast, spans);
            spans.push(ast.span_from_syn(expr_while.body.span()));
            collect_conditional_spans_from_block(&expr_while.body, ast, spans);
        }
        _ => {}
    }
}
