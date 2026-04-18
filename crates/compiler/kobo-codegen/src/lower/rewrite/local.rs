use kobo_ir::OwnershipTier;
use syn::parse_quote;

use super::super::binding::{apply_tier_to_local, binding_for_pat, binding_tier_from_expr};
use super::super::scope::{type_name_from_expr, type_name_from_syn};
use super::{LoweringAnchorKind, ScopeStack};

impl super::Lowerer<'_> {
    pub(super) fn lower_local(&mut self, local: &mut syn::Local, scopes: &mut ScopeStack) {
        super::util::strip_kobo_attrs(&mut local.attrs);
        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            self.lower_local_without_binding(local, scopes);
            return;
        };

        let tier = self.plan.tier_for_binding(binding);
        let inferred_type_name = binding
            .ty
            .as_ref()
            .and_then(type_name_from_syn)
            .or_else(|| {
                local
                    .init
                    .as_ref()
                    .and_then(|init| type_name_from_expr(init.expr.as_ref()))
            });
        self.record_binding_anchor(binding, LoweringAnchorKind::Local);
        let already_wrapped = self.lower_local_initializer(local, binding, tier, scopes);
        let diag_loc = self.plan.diag_source_loc_for(binding);
        let mutation_required = self.plan.mutation_required(binding);
        apply_tier_to_local(local, tier, already_wrapped, diag_loc, mutation_required);
        scopes.insert(&binding.ident, tier);
        if let Some(name) = inferred_type_name {
            scopes.insert_type_name(&binding.ident, name);
        }
    }

    fn lower_local_without_binding(
        &mut self,
        local: &mut syn::Local,
        scopes: &mut ScopeStack,
    ) {
        let Some(init) = &mut local.init else {
            return;
        };
        self.lower_expr(init.expr.as_mut(), scopes);
    }

    fn lower_local_initializer(
        &mut self,
        local: &mut syn::Local,
        binding: &kobo_parser::KoboBinding,
        target_tier: OwnershipTier,
        scopes: &mut ScopeStack,
    ) -> bool {
        let Some(init) = &mut local.init else {
            return false;
        };

        if self.plan.binding_uses_plain_clone_alias(binding) {
            if let Some((ident, _)) = binding_tier_from_expr(init.expr.as_ref(), scopes) {
                init.expr = Box::new(parse_quote!(#ident.clone()));
                return true;
            }
        }

        if let Some((ident, source_tier)) = binding_tier_from_expr(init.expr.as_ref(), scopes) {
            if source_tier.is_cloneable_wrapper() {
                init.expr = Box::new(parse_quote!(#ident.clone()));
                return true;
            }
            if source_tier == OwnershipTier::BoxOwned && target_tier == OwnershipTier::BoxOwned {
                return true;
            }
        }

        self.lower_expr(init.expr.as_mut(), scopes);
        false
    }
}
