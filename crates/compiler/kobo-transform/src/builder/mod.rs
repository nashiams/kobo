use std::collections::{HashMap, HashSet};

use crate::box_reason::{collect_box_reasons, type_box_reason};
use crate::classify::{binding_metadata, BindingMetadata, BindingState, TransformCtx};
use crate::clone_elision::{collect_elision_candidate_sites, AstCloneElisionCandidate};
use crate::escape::{BorrowAlias, MoveAlias};
use crate::options::TransformOptions;
use crate::small_clone::{collect_small_clone_profiles, SmallCloneProfile};
use kobo_ir::{
    BoxReason, CloneElisionCandidate, ElisionSkipReason, HintConflictFact, KirNode, KirStructDef,
    KoboSpan, MigrateSite, NodeIdGen, RelaxAttrError, TransformFacts,
};
use kobo_parser::{KoboBinding, KoboFile};

use self::helpers::{binding_ident, borrow_kind, expr_ident};

#[derive(Clone, Copy, Debug)]
struct PendingHint {
    hint: kobo_ir::OwnershipHint,
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
    pub(crate) move_aliases: Vec<MoveAlias>,
    pub(crate) clone_elision_candidates: Vec<CloneElisionCandidate>,
    pub(crate) struct_defs: Vec<KirStructDef>,
    /// Byte-offset spans of functions annotated with `#[kobo::relax]` [G5].
    pub(crate) relaxed_fn_ranges: Vec<KoboSpan>,
    /// Parse-time validation errors/warnings for `#[kobo::relax]` attributes [G5].
    pub(crate) relax_attr_errors: Vec<RelaxAttrError>,
    /// Sites tagged with `#[kobo::migrate]` — metadata-only [G6 / R05].
    pub(crate) migrate_sites: Vec<MigrateSite>,
}

pub(crate) struct TransformFactsBuilder<'a> {
    ast: &'a KoboFile,
    id_gen: &'a mut NodeIdGen,
    ctx: TransformCtx,
    nodes: Vec<KirNode>,
    transform_facts: TransformFacts,
    fact_indices: HashMap<kobo_ir::KirNodeId, usize>,
    borrow_aliases: Vec<BorrowAlias>,
    move_aliases: Vec<MoveAlias>,
    clone_elision_candidates: Vec<CloneElisionCandidate>,
    hint_conflicts: Vec<HintConflictFact>,
    function_stack: Vec<FunctionCtx>,
    box_reasons: HashMap<String, BoxReason>,
    small_clone_profiles: HashMap<String, SmallCloneProfile>,
    function_names: HashSet<String>,
    pending_elision_candidates: HashMap<kobo_ir::KoboAstNodeId, AstCloneElisionCandidate>,
    struct_defs: Vec<KirStructDef>,
    /// Byte-offset spans of functions annotated with `#[kobo::relax]` [G5].
    pub(crate) relaxed_fn_ranges: Vec<KoboSpan>,
    /// Parse-time validation errors/warnings for `#[kobo::relax]` attributes [G5].
    pub(crate) relax_attr_errors: Vec<RelaxAttrError>,
    /// Sites tagged with `#[kobo::migrate]` — metadata-only [G6 / R05].
    pub(crate) migrate_sites: Vec<MigrateSite>,
}

mod emit;
mod helpers;
mod attrs;
mod struct_collect;
#[cfg(test)]
mod tests;
mod walk;

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
    pub(crate) fn new(
        ast: &'a KoboFile,
        id_gen: &'a mut NodeIdGen,
        options: TransformOptions,
    ) -> Self {
        Self {
            ast,
            id_gen,
            ctx: TransformCtx::new(),
            nodes: Vec::new(),
            transform_facts: TransformFacts::default(),
            fact_indices: HashMap::new(),
            borrow_aliases: Vec::new(),
            move_aliases: Vec::new(),
            clone_elision_candidates: Vec::new(),
            hint_conflicts: Vec::new(),
            function_stack: Vec::new(),
            box_reasons: collect_box_reasons(ast, options.small_struct_clone_threshold_bytes),
            small_clone_profiles: collect_small_clone_profiles(
                ast,
                options.small_struct_clone_threshold_bytes,
            ),
            function_names: ast
                .inner
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Fn(function) => Some(function.sig.ident.to_string()),
                    _ => None,
                })
                .collect(),
            pending_elision_candidates: collect_elision_candidate_sites(ast)
                .into_iter()
                .map(|candidate| (candidate.alias_ast_id, candidate))
                .collect(),
            struct_defs: Vec::new(),
            relaxed_fn_ranges: Vec::new(),
            relax_attr_errors: Vec::new(),
            migrate_sites: Vec::new(),
        }
    }

    pub(crate) fn finish(self) -> BuilderOutput {
        let Self {
            nodes,
            mut transform_facts,
            borrow_aliases,
            move_aliases,
            clone_elision_candidates,
            hint_conflicts,
            struct_defs,
            relaxed_fn_ranges,
            relax_attr_errors,
            migrate_sites,
            ..
        } = self;

        for binding in &mut transform_facts.bindings {
            binding.usage.sort_uses();
        }

        let derived = kobo_ir::derive_transform_facts(
            transform_facts
                .bindings
                .iter()
                .map(|binding| binding.usage.clone())
                .collect(),
            Vec::new(),
            hint_conflicts,
        );
        transform_facts.usages = derived.usages;
        transform_facts.shared_facts = derived.shared_facts;
        transform_facts.hint_conflicts = derived.hint_conflicts;

        BuilderOutput {
            nodes,
            transform_facts,
            borrow_aliases,
            move_aliases,
            clone_elision_candidates,
            struct_defs,
            relaxed_fn_ranges,
            relax_attr_errors,
            migrate_sites,
        }
    }

    fn binding_metadata(&self, binding: &KoboBinding, init: Option<&syn::Expr>) -> BindingMetadata {
        let generic_params = self
            .function_stack
            .last()
            .map(|frame| &frame.generic_params)
            .cloned()
            .unwrap_or_default();
        let mut metadata = binding_metadata(
            binding,
            init,
            &self.ctx,
            &generic_params,
            &self.small_clone_profiles,
        );
        metadata.small_clone_profile = metadata
            .small_clone_profile
            .or_else(|| init.and_then(|expr| self.small_clone_profile_for_expr(expr)));
        if metadata.resource_kind.is_none() {
            metadata.resource_kind = helpers::detect_boxless_resource(binding, init, &self.ctx);
        }
        metadata
    }

    fn detect_box_reason(&self, binding: &KoboBinding) -> Option<BoxReason> {
        binding
            .ty
            .as_ref()
            .and_then(|ty| type_box_reason(ty, &self.box_reasons))
    }

    fn small_clone_profile_for_expr(&self, expr: &syn::Expr) -> Option<SmallCloneProfile> {
        match expr {
            syn::Expr::Struct(expr_struct) => {
                let ident = expr_struct.path.segments.last()?.ident.to_string();
                self.small_clone_profiles.get(&ident).copied()
            }
            syn::Expr::Path(_) => {
                let ident = helpers::expr_ident(expr)?;
                self.ctx.lookup(ident)?.small_clone_profile
            }
            syn::Expr::Paren(paren) => self.small_clone_profile_for_expr(paren.expr.as_ref()),
            syn::Expr::Group(group) => self.small_clone_profile_for_expr(group.expr.as_ref()),
            _ => None,
        }
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

    fn resolved_binding(&self, expr: &syn::Expr) -> Option<(BindingState, kobo_ir::KoboSpan)> {
        let ident = expr_ident(expr)?;
        let binding_state = self.ctx.lookup(ident)?;
        let span = self.ast.span_from_syn(ident.span());
        Some((binding_state, span))
    }

    fn is_local_function_expr(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
                self.function_names.contains(
                    &path
                        .path
                        .segments
                        .first()
                        .expect("single segment")
                        .ident
                        .to_string(),
                )
            }
            syn::Expr::Paren(paren) => self.is_local_function_expr(paren.expr.as_ref()),
            syn::Expr::Group(group) => self.is_local_function_expr(group.expr.as_ref()),
            _ => false,
        }
    }

    fn move_alias_from_initializer(
        &self,
        alias_decl: kobo_ir::KirNodeId,
        expr: &syn::Expr,
    ) -> Option<(MoveAlias, BindingState)> {
        let (binding_state, span) = self.resolved_binding(expr)?;
        Some((
            MoveAlias {
                alias: alias_decl,
                source: binding_state.decl_id,
                span,
                plain_clone_alias: binding_state.small_clone_profile
                    == Some(SmallCloneProfile::Eligible),
            },
            binding_state,
        ))
    }

    fn record_pending_elision_candidate(
        &mut self,
        alias_binding: &KoboBinding,
        alias_decl: kobo_ir::KirNodeId,
        source_decl: kobo_ir::KirNodeId,
    ) {
        let Some(candidate) = self
            .pending_elision_candidates
            .get(&alias_binding.id)
            .copied()
        else {
            return;
        };
        self.record_elision_candidate(CloneElisionCandidate {
            source: source_decl,
            alias: alias_decl,
            move_span: candidate.move_span,
        });
    }

    fn maybe_mark_plain_clone_alias(
        &mut self,
        alias_decl: kobo_ir::KirNodeId,
        source_state: BindingState,
        move_span: kobo_ir::KoboSpan,
        alias_is_candidate: bool,
    ) {
        if alias_is_candidate {
            return;
        }
        if source_state.small_clone_profile == Some(SmallCloneProfile::Eligible) {
            self.set_plain_clone_alias(alias_decl, source_state.decl_id, move_span);
        }
        if source_state.small_clone_profile == Some(SmallCloneProfile::OpaqueField) {
            self.set_elision_skip_reason(
                alias_decl,
                ElisionSkipReason::FieldTypeUnknownMayAllocate,
            );
        }
    }
}
