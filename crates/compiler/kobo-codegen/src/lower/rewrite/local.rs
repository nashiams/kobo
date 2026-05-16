use kobo_ir::OwnershipTier;
use syn::parse_quote;

use super::super::binding::{apply_tier_to_local, binding_for_pat, binding_tier_from_expr};
use super::super::scope::{type_name_from_expr, type_name_from_syn};
use super::{util, LoweringAnchorKind, ScopeStack};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConcurrentLocalSugar {
    Counter,
    Live,
    ViewDistance,
}

struct NumericWideningCast {
    expr: syn::Expr,
    source_type: String,
    target_type: String,
}

impl super::Lowerer<'_> {
    pub(super) fn lower_local(&mut self, local: &mut syn::Local, scopes: &mut ScopeStack) {
        let concurrent_sugar = concurrent_local_sugar(local);
        util::strip_kobo_attrs(&mut local.attrs);
        if let Some(sugar) = concurrent_sugar {
            self.lower_concurrent_sugar_local(local, sugar, scopes);
            return;
        }

        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            self.lower_local_without_binding(local, scopes);
            return;
        };

        let tier = effective_local_tier(local, self.plan.tier_for_binding(binding));
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

    fn lower_local_without_binding(&mut self, local: &mut syn::Local, scopes: &mut ScopeStack) {
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
        if let Some(cast) = numeric_widening_cast(local, scopes) {
            self.record_numeric_cast_note(binding, &cast);
            if let Some(init) = &mut local.init {
                *init.expr = cast.expr;
                return false;
            }
        }

        let Some(init) = &mut local.init else {
            return false;
        };

        if self.plan.binding_uses_plain_clone_alias(binding) {
            if let Some((ident, _)) = binding_tier_from_expr(init.expr.as_ref(), scopes) {
                *init.expr = parse_quote!(#ident.clone());
                return true;
            }
        }

        if let Some((ident, source_tier)) = binding_tier_from_expr(init.expr.as_ref(), scopes) {
            if source_tier.is_cloneable_wrapper() {
                *init.expr = parse_quote!(#ident.clone());
                return true;
            }
            if source_tier == OwnershipTier::BoxOwned && target_tier == OwnershipTier::BoxOwned {
                return true;
            }
        }

        self.lower_expr(init.expr.as_mut(), scopes);
        false
    }

    fn record_numeric_cast_note(
        &mut self,
        binding: &kobo_parser::KoboBinding,
        cast: &NumericWideningCast,
    ) {
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        let (kobo_line, _) = self.ast.line_col(binding.span);
        self.annotation_notes
            .push(super::super::plan::AnnotationNote {
                node,
                binding_name: binding.ident.to_string(),
                kobo_line,
                reason: format!(
                    "numeric-cast-debt: safe widening {} -> {} inserted for review",
                    cast.source_type, cast.target_type
                ),
            });
    }

    fn lower_concurrent_sugar_local(
        &mut self,
        local: &mut syn::Local,
        sugar: ConcurrentLocalSugar,
        scopes: &mut ScopeStack,
    ) {
        let Some(binding) = binding_for_pat(self.ast, &local.pat).cloned() else {
            self.lower_local_without_binding(local, scopes);
            return;
        };
        self.record_binding_anchor(&binding, LoweringAnchorKind::Local);

        match sugar {
            ConcurrentLocalSugar::Counter => {
                rewrite_local_type(local, parse_quote!(std::sync::atomic::AtomicU64));
                if let Some(init) = &mut local.init {
                    self.lower_expr(init.expr.as_mut(), scopes);
                    let expr = (*init.expr).clone();
                    *init.expr = parse_quote!(std::sync::atomic::AtomicU64::new(#expr));
                }
                scopes.insert_type_name(&binding.ident, "AtomicU64".to_owned());
            }
            ConcurrentLocalSugar::Live => {
                self.concurrent_support.live_cell = true;
                if let Some(original_ty) = binding.ty.clone() {
                    rewrite_local_type(local, parse_quote!(KoboArcSwap<#original_ty>));
                }
                if let Some(init) = &mut local.init {
                    self.lower_expr(init.expr.as_mut(), scopes);
                    let expr = (*init.expr).clone();
                    *init.expr = parse_quote!(KoboArcSwap::from_pointee(#expr));
                }
                scopes.insert_type_name(&binding.ident, "KoboArcSwap".to_owned());
            }
            ConcurrentLocalSugar::ViewDistance => {
                self.concurrent_support.view_distance = true;
                rewrite_local_type(local, parse_quote!(KoboViewDistance));
                if let Some(init) = &mut local.init {
                    self.lower_expr(init.expr.as_mut(), scopes);
                    let expr = (*init.expr).clone();
                    *init.expr = parse_quote!(KoboViewDistance::new(#expr));
                }
                scopes.insert_type_name(&binding.ident, "KoboViewDistance".to_owned());
            }
        }

        scopes.insert(&binding.ident, OwnershipTier::PlainOwned);
    }
}

fn effective_local_tier(local: &syn::Local, tier: OwnershipTier) -> OwnershipTier {
    if local_init_is_closure(local)
        && matches!(
            tier,
            OwnershipTier::BoxOwned
                | OwnershipTier::RcShared
                | OwnershipTier::ArcShared
                | OwnershipTier::RcMutShared
                | OwnershipTier::ArcMutShared
                | OwnershipTier::Scoped
        )
    {
        OwnershipTier::PlainOwned
    } else {
        tier
    }
}

fn local_init_is_closure(local: &syn::Local) -> bool {
    matches!(
        local.init.as_ref().map(|init| init.expr.as_ref()),
        Some(syn::Expr::Closure(_))
    )
}

fn concurrent_local_sugar(local: &syn::Local) -> Option<ConcurrentLocalSugar> {
    if util::has_kobo_attr(&local.attrs, "counter") {
        Some(ConcurrentLocalSugar::Counter)
    } else if util::has_kobo_attr(&local.attrs, "live") {
        Some(ConcurrentLocalSugar::Live)
    } else if util::has_kobo_attr(&local.attrs, "view_distance") {
        Some(ConcurrentLocalSugar::ViewDistance)
    } else {
        None
    }
}

fn rewrite_local_type(local: &mut syn::Local, ty: syn::Type) {
    if let syn::Pat::Type(typed) = &mut local.pat {
        typed.ty = Box::new(ty);
    }
}

fn numeric_widening_cast(local: &syn::Local, scopes: &ScopeStack) -> Option<NumericWideningCast> {
    let (target_ty, target_name) = local_numeric_type(local)?;
    let init = local.init.as_ref()?;
    let source_ident = expr_path_ident(init.expr.as_ref())?;
    let source_name = scopes.lookup_type_name(&source_ident)?;
    if !is_safe_numeric_widening(source_name, &target_name) {
        return None;
    }
    Some(NumericWideningCast {
        expr: parse_quote!(#source_ident as #target_ty),
        source_type: source_name.to_owned(),
        target_type: target_name,
    })
}

fn local_numeric_type(local: &syn::Local) -> Option<(syn::Type, String)> {
    let syn::Pat::Type(typed) = &local.pat else {
        return None;
    };
    let name = type_name_from_syn(typed.ty.as_ref())?;
    is_numeric_type(&name).then(|| ((*typed.ty).clone(), name))
}

fn expr_path_ident(expr: &syn::Expr) -> Option<syn::Ident> {
    match expr {
        syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            Some(path.path.segments.first()?.ident.clone())
        }
        syn::Expr::Paren(paren) => expr_path_ident(paren.expr.as_ref()),
        syn::Expr::Group(group) => expr_path_ident(group.expr.as_ref()),
        _ => None,
    }
}

fn is_numeric_type(name: &str) -> bool {
    matches!(
        name,
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    )
}

fn is_safe_numeric_widening(source: &str, target: &str) -> bool {
    numeric_rank(source).zip(numeric_rank(target)).is_some_and(
        |((source_family, source_rank), (target_family, target_rank))| {
            source_family == target_family && source_rank < target_rank
        },
    )
}

fn numeric_rank(name: &str) -> Option<(&'static str, u8)> {
    match name {
        "u8" => Some(("unsigned", 1)),
        "u16" => Some(("unsigned", 2)),
        "u32" => Some(("unsigned", 3)),
        "u64" => Some(("unsigned", 4)),
        "u128" => Some(("unsigned", 5)),
        "usize" => Some(("unsigned", 6)),
        "i8" => Some(("signed", 1)),
        "i16" => Some(("signed", 2)),
        "i32" => Some(("signed", 3)),
        "i64" => Some(("signed", 4)),
        "i128" => Some(("signed", 5)),
        "isize" => Some(("signed", 6)),
        "f32" => Some(("float", 1)),
        "f64" => Some(("float", 2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_local(stmt: syn::Stmt) -> syn::Local {
        match stmt {
            syn::Stmt::Local(local) => local,
            _ => panic!("expected local statement"),
        }
    }

    #[test]
    fn closure_bindings_remain_plain_owned() {
        let local = parse_local(parse_quote! {
            let mut add = |value: i32| { total = total + value; };
        });

        assert_eq!(
            effective_local_tier(&local, OwnershipTier::RcShared),
            OwnershipTier::PlainOwned
        );
        assert_eq!(
            effective_local_tier(&local, OwnershipTier::RcMutShared),
            OwnershipTier::PlainOwned
        );
    }

    #[test]
    fn non_closure_bindings_keep_solver_tier() {
        let local = parse_local(parse_quote! {
            let value = String::from("kobo");
        });

        assert_eq!(
            effective_local_tier(&local, OwnershipTier::RcShared),
            OwnershipTier::RcShared
        );
    }
}
