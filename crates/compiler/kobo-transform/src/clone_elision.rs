use kobo_ir::{
    BindingUsage, CloneElisionCandidate, CloneElisionDecision, ElisionFallbackReason,
    KoboAstNodeId, KoboSpan, SharedBindingFacts,
};
use kobo_parser::KoboFile;
use std::collections::HashMap;
use syn::spanned::Spanned;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AstCloneElisionCandidate {
    pub(crate) source_ast_id: KoboAstNodeId,
    pub(crate) alias_ast_id: KoboAstNodeId,
    pub(crate) move_span: KoboSpan,
}

pub(crate) fn has_use_after(usage: &BindingUsage, span: KoboSpan) -> bool {
    usage
        .uses
        .iter()
        .any(|event| event.span().start > span.start)
}

pub(crate) fn decide_clone_elision(
    candidate: Option<&CloneElisionCandidate>,
    usage: &BindingUsage,
    shared_facts: &SharedBindingFacts,
) -> (Option<CloneElisionDecision>, Option<ElisionFallbackReason>) {
    let Some(candidate) = candidate else {
        return (None, None);
    };
    let move_span = candidate.move_span;

    let no_uses_after_move = !has_use_after(usage, move_span);
    let live_borrow_at_move = shared_facts.live_borrow_at_move;

    if !no_uses_after_move || live_borrow_at_move {
        return (
            Some(CloneElisionDecision::Clone),
            Some(ElisionFallbackReason::MoveSafetyCheckFailed),
        );
    }

    if cfg!(debug_assertions) {
        debug_assert!(
            no_uses_after_move,
            "CloneElisionDecision::Move issued but original binding has uses after move site"
        );
        debug_assert!(
            !live_borrow_at_move,
            "CloneElisionDecision::Move issued but a live borrow exists at the move site"
        );
    }

    (Some(CloneElisionDecision::Move), None)
}

pub(crate) fn collect_elision_candidate_sites(ast: &KoboFile) -> Vec<AstCloneElisionCandidate> {
    let mut collector = CandidateCollector::new(ast);
    collector.collect();
    collector
        .alias_sites
        .into_iter()
        .filter(|site| {
            !collector
                .uses_by_binding
                .get(&site.source_ast_id)
                .into_iter()
                .flatten()
                .any(|span| span.start > site.move_span.start)
        })
        .collect()
}

struct CandidateCollector<'a> {
    ast: &'a KoboFile,
    scopes: Vec<HashMap<String, KoboAstNodeId>>,
    uses_by_binding: HashMap<KoboAstNodeId, Vec<KoboSpan>>,
    alias_sites: Vec<AstCloneElisionCandidate>,
}

impl<'a> CandidateCollector<'a> {
    fn new(ast: &'a KoboFile) -> Self {
        Self {
            ast,
            scopes: Vec::new(),
            uses_by_binding: HashMap::new(),
            alias_sites: Vec::new(),
        }
    }

    fn collect(&mut self) {
        for item in &self.ast.inner.items {
            self.walk_item(item);
        }
    }

    fn walk_item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Fn(function) => self.walk_function(function),
            syn::Item::Const(item_const) => self.walk_expr(item_const.expr.as_ref()),
            syn::Item::Static(item_static) => self.walk_expr(item_static.expr.as_ref()),
            _ => {}
        }
    }

    fn walk_function(&mut self, function: &syn::ItemFn) {
        self.push_scope();
        for input in &function.sig.inputs {
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            let Some(binding) = self.binding_for_pat(&argument.pat).cloned() else {
                continue;
            };
            self.define(&binding);
        }
        self.walk_block(&function.block);
        self.pop_scope();
    }

    fn walk_block(&mut self, block: &syn::Block) {
        self.push_scope();
        for statement in &block.stmts {
            self.walk_stmt(statement);
        }
        self.pop_scope();
    }

    fn walk_stmt(&mut self, statement: &syn::Stmt) {
        match statement {
            syn::Stmt::Local(local) => self.walk_local(local),
            syn::Stmt::Item(item) => self.walk_item(item),
            syn::Stmt::Expr(expr, _) => self.walk_expr(expr),
            syn::Stmt::Macro(_) => {}
        }
    }

    fn walk_local(&mut self, local: &syn::Local) {
        let binding = self.binding_for_pat(&local.pat).cloned();
        let init = local.init.as_ref().map(|init| init.expr.as_ref());
        if let (Some(alias_binding), Some(init_expr)) = (&binding, init) {
            if let Some(source_ast_id) = self.resolved_binding(init_expr) {
                self.alias_sites.push(AstCloneElisionCandidate {
                    source_ast_id,
                    alias_ast_id: alias_binding.id,
                    move_span: self.ast.span_from_syn(init_expr.span()),
                });
            }
        }

        if let Some(init_expr) = init {
            self.walk_expr(init_expr);
        }
        if let Some(binding) = binding {
            self.define(&binding);
        }
    }

    fn walk_expr(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Array(array) => {
                for element in &array.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Assign(assign) => {
                self.walk_expr(assign.left.as_ref());
                self.walk_expr(assign.right.as_ref());
            }
            syn::Expr::Binary(binary) => {
                self.walk_expr(binary.left.as_ref());
                self.walk_expr(binary.right.as_ref());
            }
            syn::Expr::Block(block) => self.walk_block(&block.block),
            syn::Expr::Call(call) => {
                self.walk_expr(call.func.as_ref());
                for argument in &call.args {
                    self.walk_expr(argument);
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
                self.walk_expr(method_call.receiver.as_ref());
                for argument in &method_call.args {
                    self.walk_expr(argument);
                }
            }
            syn::Expr::Paren(paren) => self.walk_expr(paren.expr.as_ref()),
            syn::Expr::Path(path) => self.record_path_use(path),
            syn::Expr::Reference(reference) => self.walk_expr(reference.expr.as_ref()),
            syn::Expr::Repeat(repeat) => {
                self.walk_expr(repeat.expr.as_ref());
                self.walk_expr(repeat.len.as_ref());
            }
            syn::Expr::Return(return_expr) => {
                if let Some(inner) = &return_expr.expr {
                    self.walk_expr(inner.as_ref());
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

    fn record_path_use(&mut self, path: &syn::ExprPath) {
        if path.qself.is_some() || path.path.segments.len() != 1 {
            return;
        }
        let Some(ident) = path.path.segments.first() else {
            return;
        };
        let Some(binding_id) = self.lookup(&ident.ident) else {
            return;
        };
        self.uses_by_binding
            .entry(binding_id)
            .or_default()
            .push(self.ast.span_from_syn(ident.ident.span()));
    }

    fn binding_for_pat(&self, pat: &syn::Pat) -> Option<&kobo_parser::KoboBinding> {
        let ident = pat_ident(pat)?;
        let span = self.ast.span_from_syn(ident.span());
        self.ast.binding_for_span(span)
    }

    fn resolved_binding(&self, expr: &syn::Expr) -> Option<KoboAstNodeId> {
        let ident = expr_ident(expr)?;
        self.lookup(ident)
    }

    fn define(&mut self, binding: &kobo_parser::KoboBinding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(binding.ident.to_string(), binding.id);
        }
    }

    fn lookup(&self, ident: &syn::Ident) -> Option<KoboAstNodeId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&ident.to_string()).copied())
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
}

fn pat_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => pat_ident(&typed.pat),
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

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, KoboSpan, NodeIdGen, UseEvent};
    use kobo_parser::parse_file;

    use super::*;

    fn span(start: u32) -> KoboSpan {
        KoboSpan::new(start, start + 1, FileId(0))
    }

    #[test]
    fn dead_original_assignment_uses_move() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![UseEvent::Moved { span: span(10), scope_depth: 0 }],
        };
        let shared_facts = SharedBindingFacts::default();
        let candidate = CloneElisionCandidate {
            source: kobo_ir::KirNodeId(1),
            alias: kobo_ir::KirNodeId(2),
            move_span: span(10),
        };

        let decision = decide_clone_elision(Some(&candidate), &usage, &shared_facts);

        assert_eq!(decision, (Some(CloneElisionDecision::Move), None));
    }

    #[test]
    fn later_uses_force_conservative_clone() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![
                UseEvent::Moved { span: span(10), scope_depth: 0 },
                UseEvent::ReadOnly { span: span(20) },
            ],
        };
        let shared_facts = SharedBindingFacts::default();
        let candidate = CloneElisionCandidate {
            source: kobo_ir::KirNodeId(1),
            alias: kobo_ir::KirNodeId(2),
            move_span: span(10),
        };

        let decision = decide_clone_elision(Some(&candidate), &usage, &shared_facts);

        assert_eq!(
            decision,
            (
                Some(CloneElisionDecision::Clone),
                Some(ElisionFallbackReason::MoveSafetyCheckFailed),
            )
        );
    }

    #[test]
    fn live_borrow_at_move_forces_conservative_clone() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![UseEvent::Moved { span: span(10), scope_depth: 0 }],
        };
        let shared_facts = SharedBindingFacts {
            live_borrow_at_move: true,
            ..SharedBindingFacts::default()
        };
        let candidate = CloneElisionCandidate {
            source: kobo_ir::KirNodeId(1),
            alias: kobo_ir::KirNodeId(2),
            move_span: span(10),
        };

        let decision = decide_clone_elision(Some(&candidate), &usage, &shared_facts);

        assert_eq!(
            decision,
            (
                Some(CloneElisionDecision::Clone),
                Some(ElisionFallbackReason::MoveSafetyCheckFailed),
            )
        );
    }

    #[test]
    fn bindings_without_a_candidate_do_not_produce_clone_elision_decisions() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![UseEvent::Moved { span: span(10), scope_depth: 0 }],
        };

        assert_eq!(
            decide_clone_elision(None, &usage, &SharedBindingFacts::default()),
            (None, None)
        );
    }

    #[test]
    fn subpass_a_collects_dead_original_alias_sites() {
        let source = r#"
fn main() {
    let x = String::from("hello");
    let y = x;
    println!("{}", y.len());
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");

        let candidates = collect_elision_candidate_sites(&ast);

        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn subpass_a_respects_later_uses_of_the_original_binding() {
        let source = r#"
fn main() {
    let x = String::from("hello");
    let y = x;
    x.len();
    println!("{}", y.len());
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");

        assert!(collect_elision_candidate_sites(&ast).is_empty());
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn release_builds_fall_back_to_clone_when_candidate_post_check_fails() {
        let usage = BindingUsage {
            declaration: span(0),
            uses: vec![
                UseEvent::Moved { span: span(10), scope_depth: 0 },
                UseEvent::ReadOnly { span: span(20) },
            ],
        };
        let candidate = CloneElisionCandidate {
            source: kobo_ir::KirNodeId(1),
            alias: kobo_ir::KirNodeId(2),
            move_span: span(10),
        };

        assert_eq!(
            decide_clone_elision(Some(&candidate), &usage, &SharedBindingFacts::default(),),
            (
                Some(CloneElisionDecision::Clone),
                Some(ElisionFallbackReason::MoveSafetyCheckFailed),
            )
        );
    }
}
