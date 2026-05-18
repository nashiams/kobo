mod anchor;
mod binding;
mod borrow_scope;
mod plan;
pub(crate) mod rewrite;
mod scope;
mod service;
pub(crate) mod strict;
mod support;

use std::path::Path;

use kobo_ir::SolutionMap;
use kobo_parser::KoboFile;

use self::plan::LoweringPlan;
use self::rewrite::Lowerer;
use crate::error_policy::ErrorPolicyMarker;
use crate::CodegenOptions;

#[cfg(test)]
pub(crate) use self::anchor::ResolvedAnchor;
pub(crate) use self::anchor::{
    LoweringAnchor, LoweringAnchorKind, LoweringAnchorMap, ResolvedAnchorMap,
};
pub use self::plan::{AnnotationNote, LoweringSite};

pub(crate) struct LoweredFile {
    pub(crate) file: syn::File,
    pub(crate) sites: Vec<LoweringSite>,
    pub(crate) notes: Vec<AnnotationNote>,
    pub(crate) anchors: LoweringAnchorMap,
    pub(crate) error_policy_markers: Vec<ErrorPolicyMarker>,
}

/// Formatted Rust lowering driven by the frozen KIR plus any solved ownership overrides.
pub(crate) fn lower(
    kir: &kobo_ir::Kir,
    ast: &KoboFile,
    solution: &SolutionMap,
    kobo_path: &Path,
    options: &CodegenOptions,
) -> LoweredFile {
    let mut file = ast.inner.clone();
    let service_support_items = service::service_support_items(ast, &file);
    let plan = LoweringPlan::from_kir(ast, kir, solution, kobo_path, options);
    let mut lowerer = Lowerer::new(ast, &plan, kir, options);
    lowerer.lower_items(&mut file.items);
    plan.insert_support_items(&mut file);
    let (mut lowerer_notes, lowerer_anchors, error_policy_markers, concurrent_support) =
        lowerer.into_parts();
    support::insert_concurrent_support_items(&mut file, concurrent_support);
    service::append_service_support_items(&mut file, service_support_items);
    let mut notes = plan.annotation_notes().to_vec();
    notes.append(&mut lowerer_notes);
    let anchors = LoweringAnchorMap::new(lowerer_anchors, plan.support_item_count());

    LoweredFile {
        file,
        sites: plan.annotation_sites().to_vec(),
        notes,
        anchors,
        error_policy_markers,
    }
}
