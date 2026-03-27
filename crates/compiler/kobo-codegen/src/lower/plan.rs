use std::collections::HashMap;

use kobo_ir::{KoboAstNodeId, OwnershipTier, SolutionMap};
use kobo_parser::{KoboBinding, KoboFile};

use super::binding::binding_for_pat;
use super::support::support_items;

pub(crate) struct LoweringPlan {
    tiers_by_ast: HashMap<KoboAstNodeId, OwnershipTier>,
    function_param_tiers: HashMap<String, Vec<OwnershipTier>>,
    needs_rc_refcell: bool,
    needs_scoped_handle: bool,
}

impl LoweringPlan {
    pub(crate) fn from_kir(ast: &KoboFile, kir: &kobo_ir::Kir, solution: &SolutionMap) -> Self {
        let mut tiers_by_ast = HashMap::new();
        let mut needs_rc_refcell = false;
        let mut needs_scoped_handle = false;

        for node in kir.iter_nodes() {
            let resolved_tier = solution.resolve(node.id, node.ownership);
            tiers_by_ast.insert(node.ast_id, resolved_tier);
            needs_rc_refcell |= resolved_tier == OwnershipTier::RcMutShared;
            needs_scoped_handle |= resolved_tier == OwnershipTier::Scoped;
        }

        let function_param_tiers = build_function_param_tiers(ast, &tiers_by_ast);

        Self {
            tiers_by_ast,
            function_param_tiers,
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
