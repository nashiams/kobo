/// K0041: active alias detection for RcMutShared bindings.
///
/// Walks the enclosing function body before @strict block entry. Detects:
/// 1. `.clone()` calls on the binding → alias via Rc::clone
/// 2. `let alias = binding;` → alias via move/copy
/// 3. Function calls passing binding by value → alias escapes
///
/// Contract C03: produces StrictBoundaryViolation::ActiveAliases (never Warning).
/// Contract C05: produces facts, NOT KDiagnostic.
use kobo_ir::{
    CaptureSet, Kir, KirNodeId, KoboSpan, OwnershipTier,
    StrictBoundaryFact, StrictBoundaryViolation,
};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

use super::span_convert::SpanConvert;

pub(super) fn check_k0041_active_aliases(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    kir: &Kir,
    enclosing_stmts: &[syn::Stmt],
    facts: &mut Vec<StrictBoundaryFact>,
    sc: &SpanConvert,
) {
    if !block.is_strict || capture_set.bindings.is_empty() {
        return;
    }

    let rc_mut_names: std::collections::HashSet<&str> = capture_set
        .bindings
        .iter()
        .filter(|b| {
            kir.tier_decision(b.binding_id)
                .map(|td| td.tier == OwnershipTier::RcMutShared)
                .unwrap_or(false)
        })
        .map(|b| b.name.as_str())
        .collect();

    if rc_mut_names.is_empty() {
        return;
    }

    let mut alias_visitor = AliasScanVisitor::new(&rc_mut_names, sc);

    for stmt in enclosing_stmts {
        alias_visitor.visit_stmt(stmt);
    }

    for (binding_name, alias_sites) in alias_visitor.aliases {
        if alias_sites.is_empty() {
            continue;
        }
        let binding_id = capture_set
            .bindings
            .iter()
            .find(|b| b.name == binding_name)
            .map(|b| b.binding_id)
            .unwrap_or(KirNodeId(0));

        facts.push(StrictBoundaryFact {
            block_span: capture_set.block_span,
            violation: StrictBoundaryViolation::ActiveAliases {
                binding_id,
                alias_sites,
            },
        });
    }
}

// --- AliasScanVisitor ---

struct AliasScanVisitor<'a> {
    rc_mut_names: &'a std::collections::HashSet<&'a str>,
    aliases: std::collections::HashMap<String, Vec<KoboSpan>>,
    sc: &'a SpanConvert,
}

impl<'a> AliasScanVisitor<'a> {
    fn new(rc_mut_names: &'a std::collections::HashSet<&'a str>, sc: &'a SpanConvert) -> Self {
        Self {
            rc_mut_names,
            aliases: std::collections::HashMap::new(),
            sc,
        }
    }

    fn record_alias(&mut self, name: &str, node_span: proc_macro2::Span) {
        let alias_span = self.sc.span(node_span);
        self.aliases
            .entry(name.to_string())
            .or_default()
            .push(alias_span);
    }

    fn is_name_match(&self, expr: &syn::Expr) -> Option<String> {
        if let syn::Expr::Path(path) = expr {
            if let Some(ident) = path.path.get_ident() {
                let name = ident.to_string();
                if self.rc_mut_names.contains(name.as_str()) {
                    return Some(name);
                }
            }
        }
        None
    }
}

impl<'a, 'ast> Visit<'ast> for AliasScanVisitor<'a> {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "clone" {
            if let Some(name) = self.is_name_match(&node.receiver) {
                use syn::spanned::Spanned;
                self.record_alias(&name, node.span());
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some(init) = &node.init {
            if let Some(name) = self.is_name_match(&init.expr) {
                use syn::spanned::Spanned;
                self.record_alias(&name, node.span());
            }
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        for arg in &node.args {
            if let Some(name) = self.is_name_match(arg) {
                use syn::spanned::Spanned;
                self.record_alias(&name, node.span());
            }
        }
        syn::visit::visit_expr_call(self, node);
    }
}
