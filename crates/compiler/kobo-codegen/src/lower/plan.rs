use std::collections::HashMap;

use kobo_ir::{KoboAstNodeId, KoboSpan, NodeKind, OwnershipTier, ResourceKind, SolutionMap};
use kobo_parser::{KoboBinding, KoboFile};

use super::binding::binding_for_pat;
use super::support::support_items;

#[derive(Clone, Debug)]
pub struct LoweringSite {
    pub binding_name: String,
    pub ownership_tier: OwnershipTier,
    pub kobo_span: KoboSpan,
    pub kobo_line: usize,
    pub reason: String,
}

pub(crate) struct LoweringPlan {
    tiers_by_ast: HashMap<KoboAstNodeId, OwnershipTier>,
    function_param_tiers: HashMap<String, Vec<OwnershipTier>>,
    annotation_sites: Vec<LoweringSite>,
    needs_rc_refcell: bool,
    needs_scoped_handle: bool,
}

impl LoweringPlan {
    pub(crate) fn from_kir(ast: &KoboFile, kir: &kobo_ir::Kir, solution: &SolutionMap) -> Self {
        let mut tiers_by_ast = HashMap::new();
        let mut resource_kinds_by_ast = HashMap::new();
        let mut needs_rc_refcell = false;
        let mut needs_scoped_handle = false;

        for node in kir.iter_decl_nodes() {
            debug_assert_eq!(node.kind, NodeKind::Decl);
            let resolved_tier = solution.resolve(node.id, node.ownership);
            let ast_id = node.ast_id.expect("declaration nodes must carry KoboAstNodeId");
            tiers_by_ast.insert(ast_id, resolved_tier);
            resource_kinds_by_ast.insert(ast_id, node.resource_kind);
            needs_rc_refcell |= resolved_tier == OwnershipTier::RcMutShared;
            needs_scoped_handle |= resolved_tier == OwnershipTier::Scoped;
        }

        let function_param_tiers = build_function_param_tiers(ast, &tiers_by_ast);
        let annotation_sites = build_annotation_sites(ast, &tiers_by_ast, &resource_kinds_by_ast);

        Self {
            tiers_by_ast,
            function_param_tiers,
            annotation_sites,
            needs_rc_refcell,
            needs_scoped_handle,
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
        let mut prepended_items = support_items(self.needs_rc_refcell, self.needs_scoped_handle);
        if prepended_items.is_empty() {
            return;
        }

        prepended_items.append(&mut file.items);
        file.items = prepended_items;
    }

    pub(crate) fn annotation_sites(&self) -> &[LoweringSite] {
        &self.annotation_sites
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
    tiers_by_ast: &HashMap<KoboAstNodeId, OwnershipTier>,
    resource_kinds_by_ast: &HashMap<KoboAstNodeId, Option<ResourceKind>>,
) -> Vec<LoweringSite> {
    let mut sites = Vec::new();

    for binding in ast.iter_bindings() {
        let ownership_tier = *tiers_by_ast
            .get(&binding.id)
            .unwrap_or(&OwnershipTier::PlainOwned);
        if !should_annotate(ownership_tier) {
            continue;
        }

        let resource_kind = resource_kinds_by_ast
            .get(&binding.id)
            .copied()
            .flatten();
        let (kobo_line, _) = ast.line_col(binding.span);
        sites.push(LoweringSite {
            binding_name: binding.ident.to_string(),
            ownership_tier,
            kobo_span: binding.span,
            kobo_line,
            reason: annotation_reason(ownership_tier, resource_kind).to_owned(),
        });
    }

    sites
}

fn should_annotate(tier: OwnershipTier) -> bool {
    !matches!(tier, OwnershipTier::PlainOwned)
}

fn annotation_reason(tier: OwnershipTier, resource_kind: Option<ResourceKind>) -> &'static str {
    if resource_kind.is_some() || tier == OwnershipTier::Scoped {
        return "resource wrapper";
    }

    match tier {
        OwnershipTier::BoxOwned => "single-owner heap allocation",
        OwnershipTier::RcShared => "shared read access",
        OwnershipTier::ArcShared => "thread-safe shared read access",
        OwnershipTier::RcMutShared => "non-Copy shared binding",
        OwnershipTier::ArcMutShared => "thread-safe mutable sharing",
        OwnershipTier::Scoped => "resource wrapper",
        OwnershipTier::Undecided => "compiler-owned wrapper",
        OwnershipTier::PlainOwned => "unchanged binding",
    }
}
