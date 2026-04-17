mod span_walk;
mod types;

pub use types::{AnnotationNote, LoweringSite};

use std::collections::{HashMap, HashSet};
use std::path::Path;

use kobo_ir::{
    KirNodeId, KoboAstNodeId, KoboSpan, NodeKind, OwnershipTier, SatisfactionCheck, SolutionMap,
    TierReason,
};
use kobo_parser::{KoboBinding, KoboFile};

use crate::CodegenOptions;
use super::binding::binding_for_pat;
use super::support::support_items;

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
    needs_diag_owner: bool,
    diag_mode: bool,
    /// Precomputed source-location strings (`"file.kobo:line"`) for each
    /// `RcMutShared` AST binding that will be wrapped in `DiagOwner`.
    diag_source_locs: HashMap<KoboAstNodeId, String>,
    /// S-1: AST bindings that need `let mut` (local-only mutation).
    mutation_required_bindings: HashSet<KoboAstNodeId>,
    support_item_count: usize,
}

impl LoweringPlan {
    pub(crate) fn from_kir(
        ast: &KoboFile,
        kir: &kobo_ir::Kir,
        solution: &SolutionMap,
        kobo_path: &Path,
        options: &CodegenOptions,
    ) -> Self {
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
        let mut annotation_sites = build_annotation_sites(ast, kir, &annotation_tiers_by_ast);
        let annotation_notes = build_annotation_notes(ast, kir);
        let plain_clone_aliases = kir
            .transform_facts()
            .iter_bindings()
            .filter(|binding| binding.plain_clone_alias)
            .map(|binding| binding.node)
            .collect();

        // Compute source-location strings for DiagOwner wrapping.
        let needs_diag_owner = options.diag_mode && needs_refcell;
        let diag_source_locs: HashMap<KoboAstNodeId, String> = if options.diag_mode {
            let file_name = kobo_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.kobo");
            lowering_tiers_by_ast
                .iter()
                .filter(|(_, &tier)| tier == OwnershipTier::RcMutShared)
                .filter_map(|(&ast_id, _)| {
                    let binding = ast.binding_for_id(ast_id)?;
                    let (line, _col) = ast.line_col(binding.span);
                    // Lines in line_col are 0-indexed internally; display as 1-indexed.
                    let loc = format!("{}:{}", file_name, line + 1);
                    Some((ast_id, loc))
                })
                .collect()
        } else {
            HashMap::new()
        };

        let support_item_count =
            support_items(needs_rc, needs_refcell, needs_arc, needs_scoped_handle, needs_diag_owner)
                .len();

        // Mark annotation sites that will be wrapped in DiagOwner.
        if options.diag_mode {
            let diag_node_ids: HashSet<KirNodeId> = diag_source_locs
                .keys()
                .filter_map(|ast_id| nodes_by_ast.get(ast_id).copied())
                .collect();
            for site in &mut annotation_sites {
                if diag_node_ids.contains(&site.node) {
                    site.diag_wrapped = true;
                }
            }
        }

        // S-1: collect bindings that need `let mut` (local-only mutation at PlainOwned).
        let mutation_required_bindings: HashSet<KoboAstNodeId> = nodes_by_ast
            .iter()
            .filter_map(|(&ast_id, &kir_id)| {
                let tf = kir.transform_facts().bindings.iter().find(|b| b.node == kir_id)?;
                if tf.shared_facts.mutation_required {
                    Some(ast_id)
                } else {
                    None
                }
            })
            .collect();

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
            needs_diag_owner,
            diag_mode: options.diag_mode,
            diag_source_locs,
            mutation_required_bindings,
            support_item_count,
        }
    }

    /// Return the precomputed `DiagOwner` source-location string for a binding,
    /// or `None` if `diag_mode` is off or the binding is not `RcMutShared`.
    pub(crate) fn diag_source_loc_for(&self, binding: &KoboBinding) -> Option<&str> {
        if !self.diag_mode {
            return None;
        }
        self.diag_source_locs.get(&binding.id).map(String::as_str)
    }

    pub(crate) fn tier_for_binding(&self, binding: &KoboBinding) -> OwnershipTier {
        *self
            .lowering_tiers_by_ast
            .get(&binding.id)
            .unwrap_or(&OwnershipTier::PlainOwned)
    }

    pub(crate) fn mutation_required(&self, binding: &KoboBinding) -> bool {
        self.mutation_required_bindings.contains(&binding.id)
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
            self.needs_diag_owner,
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
            diag_wrapped: false,
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
    let conditional_body_spans = span_walk::collect_conditional_body_spans(ast);

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
