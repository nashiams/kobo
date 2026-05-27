mod anchors;
pub(crate) mod async_wrapper;
mod attrs;
pub(crate) mod clone_inject;
mod expr;
mod field_capability;
mod functions;
mod items;
mod local;
pub(crate) mod lock_order;
mod parallel_gate;
mod receivers;
pub(crate) mod spawn;
mod spawn_captures;
pub(crate) mod spawn_strategy;
pub(crate) mod split_borrow;
mod statements;
pub(super) mod syntax_support;
pub(crate) mod tick;
pub(super) use syntax_support as util;

#[cfg(test)]
mod tests;

use kobo_parser::KoboFile;
use quote::ToTokens;
use syn::parse_quote;
use syn::spanned::Spanned;

use super::binding::{apply_tier_to_fn_arg_type, binding_for_pat, fn_arg_lowering_tier};
use super::borrow_scope::{has_later_alias_use, rewritable_method_call, simple_borrow_alias};
use super::handler;
use super::parallel;
use super::plan::{AnnotationNote, LoweringPlan};
use super::scope::{type_name_from_syn, ScopeStack};
use super::strict::StrictGuardCounter;
use super::{LoweringAnchor, LoweringAnchorKind};
use crate::error_policy::ErrorPolicyMarker;
use crate::{CodegenOptions, ParallelLoopEvidence, TaskLocalEvidence};

#[cfg(test)]
use attrs::{executor_main_attr, is_executor_main_attr};
#[cfg(test)]
use functions::wrap_function_body_in_local_set;
#[cfg(test)]
use spawn_captures::captured_bindings_need_spawn_local;
pub(crate) struct Lowerer<'a> {
    pub(super) ast: &'a KoboFile,
    pub(super) plan: &'a LoweringPlan,
    pub(super) kir: &'a kobo_ir::Kir,
    pub(super) options: &'a CodegenOptions,
    pub(super) in_async_context: bool,
    pub(super) needs_local_set: bool,
    pub(super) strict_counter: StrictGuardCounter,
    pub(super) iter_snapshot_counter: usize,
    pub(super) concurrent_support: ConcurrentSupportNeeds,
    pub(super) annotation_notes: Vec<AnnotationNote>,
    pub(super) anchors: Vec<LoweringAnchor>,
    pub(super) error_policy_markers: Vec<ErrorPolicyMarker>,
    pub(super) needs_rayon: bool,
    pub(super) parallel_evidence: Vec<ParallelLoopEvidence>,
    pub(super) task_local_evidence: Vec<TaskLocalEvidence>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ConcurrentSupportNeeds {
    pub(crate) live_cell: bool,
    pub(crate) view_distance: bool,
}

impl<'a> Lowerer<'a> {
    pub(crate) fn new(
        ast: &'a KoboFile,
        plan: &'a LoweringPlan,
        kir: &'a kobo_ir::Kir,
        options: &'a CodegenOptions,
    ) -> Self {
        Self {
            ast,
            plan,
            kir,
            options,
            in_async_context: false,
            needs_local_set: false,
            strict_counter: StrictGuardCounter::new(),
            iter_snapshot_counter: 0,
            concurrent_support: ConcurrentSupportNeeds::default(),
            annotation_notes: Vec::new(),
            anchors: Vec::new(),
            error_policy_markers: Vec::new(),
            needs_rayon: false,
            parallel_evidence: Vec::new(),
            task_local_evidence: Vec::new(),
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<AnnotationNote>,
        Vec<LoweringAnchor>,
        Vec<ErrorPolicyMarker>,
        ConcurrentSupportNeeds,
        bool,
        Vec<ParallelLoopEvidence>,
        Vec<TaskLocalEvidence>,
    ) {
        (
            self.annotation_notes,
            self.anchors,
            self.error_policy_markers,
            self.concurrent_support,
            self.needs_rayon,
            self.parallel_evidence,
            self.task_local_evidence,
        )
    }
}
