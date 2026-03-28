use std::collections::HashMap;

use kobo_ir::{KirNodeId, KoboAstNodeId, KoboSpan, NodeKind, OwnershipTier, SolutionMap};
use kobo_parser::{KoboBinding, KoboFile};

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
    tiers_by_ast: HashMap<KoboAstNodeId, OwnershipTier>,
    function_param_tiers: HashMap<String, Vec<OwnershipTier>>,
    annotation_sites: Vec<LoweringSite>,
    annotation_notes: Vec<AnnotationNote>,
    needs_rc: bool,
    needs_refcell: bool,
    needs_arc: bool,
    needs_scoped_handle: bool,
    support_item_count: usize,
}

impl LoweringPlan {
    pub(crate) fn from_kir(ast: &KoboFile, kir: &kobo_ir::Kir, solution: &SolutionMap) -> Self {
        let mut nodes_by_ast = HashMap::new();
        let mut tiers_by_ast = HashMap::new();
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
            nodes_by_ast.insert(ast_id, node.id);
            tiers_by_ast.insert(ast_id, resolved_tier);

            needs_rc |= matches!(
                resolved_tier,
                OwnershipTier::RcShared | OwnershipTier::RcMutShared
            );
            needs_refcell |= resolved_tier == OwnershipTier::RcMutShared;
            needs_arc |= matches!(
                resolved_tier,
                OwnershipTier::ArcShared | OwnershipTier::ArcMutShared
            );
            needs_scoped_handle |= resolved_tier == OwnershipTier::Scoped;
        }

        let function_param_tiers = build_function_param_tiers(ast, &tiers_by_ast);
        let annotation_sites = build_annotation_sites(ast, kir, &tiers_by_ast);
        let annotation_notes = build_annotation_notes(ast, kir);
        let support_item_count = support_items(
            needs_rc,
            needs_refcell,
            needs_arc,
            needs_scoped_handle,
        )
        .len();

        Self {
            nodes_by_ast,
            tiers_by_ast,
            function_param_tiers,
            annotation_sites,
            annotation_notes,
            needs_rc,
            needs_refcell,
            needs_arc,
            needs_scoped_handle,
            support_item_count,
        }
    }

    pub(crate) fn tier_for_binding(&self, binding: &KoboBinding) -> OwnershipTier {
        *self
            .tiers_by_ast
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

fn build_annotation_notes(ast: &KoboFile, kir: &kobo_ir::Kir) -> Vec<AnnotationNote> {
    let mut notes = Vec::new();

    for binding in kir.transform_facts().iter_bindings() {
        let Some(fallback_reason) = binding.elision_fallback else {
            continue;
        };
        let Some(ast_binding) = ast.binding_for_id(binding.ast_id) else {
            continue;
        };

        let (kobo_line, _) = ast.line_col(ast_binding.span);
        notes.push(AnnotationNote {
            node: binding.node,
            binding_name: ast_binding.ident.to_string(),
            kobo_line,
            reason: kobo_ir::TierReason::CloneElisionFallback(fallback_reason)
                .annotation_text(OwnershipTier::PlainOwned),
        });
    }

    notes
}
