pub(crate) mod async_wrapper;
pub(crate) mod clone_inject;
mod expr;
mod local;
pub(crate) mod lock_order;
pub(crate) mod spawn;
pub(crate) mod spawn_strategy;
pub(crate) mod split_borrow;
pub(crate) mod tick;
mod util;

use kobo_parser::KoboFile;
use syn::parse_quote;

use super::binding::{apply_tier_to_fn_arg_type, binding_for_pat, fn_arg_lowering_tier};
use super::borrow_scope::{has_later_alias_use, rewritable_method_call, simple_borrow_alias};
use super::handler;
use super::parallel;
use super::plan::{AnnotationNote, LoweringPlan};
use super::scope::{type_name_from_syn, ScopeStack};
use super::strict::StrictGuardCounter;
use super::{LoweringAnchor, LoweringAnchorKind};
use crate::error_policy::ErrorPolicyMarker;
use crate::executor::executor_attribute;
use crate::CodegenOptions;

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
        }
    }

    pub(crate) fn lower_items(&mut self, items: &mut [syn::Item]) {
        for item in items {
            self.lower_item(item);
        }
    }

    /// S-53: Check whether any captured binding has a non-Send ownership tier.
    /// When true, the spawn block should use `spawn_local` instead of `tokio::spawn`.
    fn any_captured_non_send(&self, captured: &[clone_inject::CapturedBinding]) -> bool {
        captured_bindings_need_spawn_local(captured)
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<AnnotationNote>,
        Vec<LoweringAnchor>,
        Vec<ErrorPolicyMarker>,
        ConcurrentSupportNeeds,
        bool,
    ) {
        (
            self.annotation_notes,
            self.anchors,
            self.error_policy_markers,
            self.concurrent_support,
            self.needs_rayon,
        )
    }

    fn lower_item(&mut self, item: &mut syn::Item) {
        match item {
            syn::Item::Fn(function) => {
                self.apply_executor_attribute(function);
                // P5: check if this is an @strict fn (by span matching against ast.strict_fns()).
                use syn::spanned::Spanned;
                let fn_span = self.ast.span_from_syn(function.span());
                if let Some(kfn) = self
                    .ast
                    .strict_fns()
                    .iter()
                    .find(|f| f.span == fn_span)
                    .cloned()
                {
                    let fn_body_span = self.ast.span_from_syn(function.block.span());
                    let cap = self
                        .kir
                        .strict_capture_sets()
                        .iter()
                        .find(|cs| cs.block_span == fn_body_span)
                        .cloned();
                    let mode = self
                        .kir
                        .strict_fn_modes()
                        .get(&kfn.span)
                        .copied()
                        .unwrap_or(kobo_ir::StrictFnMode::Full);
                    let ts = super::strict::lower_strict_fn(
                        &kfn,
                        mode,
                        cap.as_ref(),
                        &mut self.strict_counter,
                        self.options,
                    );
                    if let Ok(lowered) = syn::parse2::<syn::ItemFn>(ts) {
                        *function = lowered;
                    } else {
                        self.lower_function(function);
                    }
                } else {
                    self.lower_function(function);
                }
            }
            syn::Item::Const(item_const) => {
                self.record_item_anchor(&item_const.ident, LoweringAnchorKind::Const)
            }
            syn::Item::Static(item_static) => {
                self.record_item_anchor(&item_static.ident, LoweringAnchorKind::Static)
            }
            syn::Item::Struct(item_struct) => {
                self.lower_struct_item(item_struct);
            }
            syn::Item::Impl(item_impl) => self.lower_impl_block(item_impl),
            syn::Item::Type(item_type) => {
                util::strip_kobo_attrs(&mut item_type.attrs);
            }
            syn::Item::Enum(item_enum) => {
                util::strip_kobo_attrs(&mut item_enum.attrs);
            }
            _ => {}
        }
    }

    fn lower_struct_item(&mut self, item_struct: &mut syn::ItemStruct) {
        util::strip_kobo_attrs(&mut item_struct.attrs);
        for field in &mut item_struct.fields {
            if util::has_kobo_attr(&field.attrs, "counter") {
                field.ty = parse_quote!(std::sync::atomic::AtomicU64);
            } else if util::has_kobo_attr(&field.attrs, "live") {
                self.concurrent_support.live_cell = true;
                let original_ty = field.ty.clone();
                field.ty = parse_quote!(KoboArcSwap<#original_ty>);
            } else if util::has_kobo_attr(&field.attrs, "view_distance") {
                self.concurrent_support.view_distance = true;
                field.ty = parse_quote!(KoboViewDistance);
            }
            util::strip_kobo_attrs(&mut field.attrs);
        }
    }

    fn apply_executor_attribute(&self, function: &mut syn::ItemFn) {
        let is_main = function.sig.asyncness.is_some() && function.sig.ident == "main";
        let Some(attr) = executor_main_attr(self.options.executor_choice, is_main) else {
            return;
        };
        if function.attrs.iter().any(is_executor_main_attr) {
            return;
        }
        function.attrs.insert(0, attr);
    }

    fn lower_impl_block(&mut self, item_impl: &mut syn::ItemImpl) {
        util::strip_kobo_attrs(&mut item_impl.attrs);

        // S-20: Detect split-borrow sites and apply destructuring.
        let items_snapshot: Vec<syn::Item> = vec![syn::Item::Impl(item_impl.clone())];
        let split_sites = kobo_analysis::split_borrow::detect_split_borrow_sites(&items_snapshot);

        for impl_item in &mut item_impl.items {
            if let syn::ImplItem::Fn(method) = impl_item {
                // Apply split-borrow rewrite if a site was detected for this method.
                if let Some(site) = split_sites
                    .iter()
                    .find(|s| method.sig.ident == s.method_name)
                {
                    split_borrow::generate_split_borrow(method, site);
                }
                self.lower_impl_method(method);
            }
        }
    }

    fn lower_impl_method(&mut self, method: &mut syn::ImplItemFn) {
        util::strip_kobo_attrs(&mut method.attrs);
        self.strict_counter = StrictGuardCounter::new();
        let prior_async_context = self.in_async_context;
        self.in_async_context = method.sig.asyncness.is_some();
        let mut scopes = ScopeStack::new();
        scopes.push();
        self.lower_method_params(&mut method.sig.inputs, &mut scopes);
        self.lower_block_statements(&mut method.block, &mut scopes);
        scopes.pop();
        self.in_async_context = prior_async_context;
    }

    fn lower_method_params(
        &mut self,
        inputs: &mut syn::punctuated::Punctuated<syn::FnArg, syn::Token![,]>,
        scopes: &mut ScopeStack,
    ) {
        for input in inputs {
            match input {
                syn::FnArg::Receiver(_) => {
                    // &self / &mut self — pass through unchanged.
                }
                syn::FnArg::Typed(argument) => {
                    util::strip_kobo_attrs(&mut argument.attrs);
                    let Some(binding) = binding_for_pat(self.ast, &argument.pat) else {
                        continue;
                    };
                    let tier = fn_arg_lowering_tier(self.plan.tier_for_binding(binding));
                    self.record_binding_anchor(binding, LoweringAnchorKind::Parameter);
                    apply_tier_to_fn_arg_type(argument, tier);
                    scopes.insert(&binding.ident, tier);
                    if let Some(ty) = &binding.ty {
                        if let Some(name) = type_name_from_syn(ty) {
                            scopes.insert_type_name(&binding.ident, name);
                        }
                    }
                }
            }
        }
    }

    fn lower_function(&mut self, function: &mut syn::ItemFn) {
        let lowered_handler = handler::lower_item_function(self.ast, function);
        // S-16: #[kobo::tick(rate=N)] → inject interval loop before lowering body.
        let tick_rate = function.attrs.iter().find_map(tick::parse_tick_rate);
        if let Some(rate) = tick_rate {
            // Remove the tick attribute.
            function.attrs.retain(|a| !tick::is_tick_attribute(a));
            // Make the function async.
            function.sig.asyncness = Some(syn::token::Async::default());
            // Inject the interval loop wrapping the original body.
            let interval_ms = if rate > 0 {
                1000u64 / rate as u64
            } else {
                1000u64
            };
            let original_stmts = std::mem::take(&mut function.block.stmts);
            let preamble: Vec<syn::Stmt> = syn::parse_quote! {
                use std::time::Duration;
                use tokio::time::MissedTickBehavior;
                let mut __kobo_interval = tokio::time::interval(Duration::from_millis(#interval_ms));
                __kobo_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            };
            let loop_body: syn::Stmt = syn::parse_quote! {
                loop {
                    __kobo_interval.tick().await;
                    #(#original_stmts)*
                }
            };
            function.block.stmts = preamble;
            function.block.stmts.push(loop_body);
        }
        // S-10: #[kobo::handler] → wrap body in per-request isolation boundary.
        let is_handler = !lowered_handler
            && function.attrs.iter().any(|attr| {
                let segments: Vec<_> = attr.path().segments.iter().collect();
                segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "handler"
            });
        if is_handler {
            // S-56: Per-request isolation — clone Arc params into locals, then wrap in catch_unwind.
            let original_stmts = std::mem::take(&mut function.block.stmts);

            // Generate clone statements for Arc-typed parameters.
            let mut clone_stmts: Vec<syn::Stmt> = Vec::new();
            for param in &function.sig.inputs {
                if let syn::FnArg::Typed(pat_type) = param {
                    let ty_str = quote::quote!(#pat_type.ty).to_string();
                    if ty_str.contains("Arc") {
                        if let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                            let ident = &pat_ident.ident;
                            let clone_stmt: syn::Stmt = parse_quote! {
                                let #ident = #ident.clone();
                            };
                            clone_stmts.push(clone_stmt);
                        }
                    }
                }
            }

            let return_ty_is_result = match &function.sig.output {
                syn::ReturnType::Type(_, ty) => {
                    let ty_str = quote::quote!(#ty).to_string();
                    ty_str.contains("Result")
                }
                _ => false,
            };
            if return_ty_is_result {
                let wrapped: Vec<syn::Stmt> = syn::parse_quote! {
                    let __kobo_handler_result = std::panic::AssertUnwindSafe(async move {
                        #(#original_stmts)*
                    });
                    match std::panic::catch_unwind(|| {}) {
                        Ok(()) => __kobo_handler_result.await,
                        Err(_panic) => {
                            eprintln!("kobo: handler panicked — request isolated");
                            Err("handler panicked".into())
                        }
                    }
                };
                function.block.stmts = clone_stmts;
                function.block.stmts.extend(wrapped);
            } else {
                let wrapped: Vec<syn::Stmt> = syn::parse_quote! {
                    let __kobo_handler_body = async move {
                        #(#original_stmts)*
                    };
                    __kobo_handler_body.await
                };
                function.block.stmts = clone_stmts;
                function.block.stmts.extend(wrapped);
            }
        }
        util::strip_kobo_attrs(&mut function.attrs);
        // Reset guard counter per function (Contract C08 / Trap 16).
        self.strict_counter = StrictGuardCounter::new();
        let prior_async_context = self.in_async_context;
        let prior_needs_local_set = self.needs_local_set;
        self.in_async_context = function.sig.asyncness.is_some();
        self.needs_local_set = false;
        let mut scopes = ScopeStack::new();
        scopes.push();
        self.lower_function_params(&mut function.sig.inputs, &mut scopes);
        self.lower_block_statements(&mut function.block, &mut scopes);
        self.inject_field_capability_borrows(function);
        scopes.pop();
        let needs_local_set = self.needs_local_set;
        self.in_async_context = prior_async_context;
        self.needs_local_set = prior_needs_local_set;
        if needs_local_set {
            wrap_function_body_in_local_set(function);
        }
    }

    fn inject_field_capability_borrows(&self, function: &mut syn::ItemFn) {
        let mut preamble = Vec::new();
        for view in self
            .kir
            .field_capability_views()
            .iter()
            .filter(|view| view.function == function.sig.ident.to_string())
        {
            let Some(owner) = ident_from_kobo(&view.owner) else {
                continue;
            };
            for field in &view.fields {
                let Some(field_ident) = ident_from_kobo(&field.name) else {
                    continue;
                };
                let Some(binding) =
                    ident_from_kobo(&format!("_kobo_using_{}_{}", view.owner, field.name))
                else {
                    continue;
                };
                let stmt: syn::Stmt = if field.mutable {
                    parse_quote!({
                        let #binding = &mut #owner.#field_ident;
                    })
                } else {
                    parse_quote!({
                        let #binding = &#owner.#field_ident;
                    })
                };
                preamble.push(stmt);
            }
        }

        if preamble.is_empty() {
            return;
        }

        let original = std::mem::take(&mut function.block.stmts);
        preamble.extend(original);
        function.block.stmts = preamble;
    }

    fn lower_function_params(
        &mut self,
        inputs: &mut syn::punctuated::Punctuated<syn::FnArg, syn::Token![,]>,
        scopes: &mut ScopeStack,
    ) {
        for input in inputs {
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            // Strip #[kobo::...] attributes from parameters [BUG-03 / Trap 17].
            util::strip_kobo_attrs(&mut argument.attrs);
            let Some(binding) = binding_for_pat(self.ast, &argument.pat) else {
                continue;
            };

            let tier = fn_arg_lowering_tier(self.plan.tier_for_binding(binding));
            self.record_binding_anchor(binding, LoweringAnchorKind::Parameter);
            apply_tier_to_fn_arg_type(argument, tier);
            scopes.insert(&binding.ident, tier);
            if let Some(ty) = &binding.ty {
                if let Some(name) = type_name_from_syn(ty) {
                    scopes.insert_type_name(&binding.ident, name);
                }
            }
        }
    }

    pub(super) fn lower_block_statements(
        &mut self,
        block: &mut syn::Block,
        scopes: &mut ScopeStack,
    ) {
        let mut index = 0usize;
        while index < block.stmts.len() {
            if self.try_materialize_iterator_mutation(block, index, scopes) {
                self.lower_stmt(&mut block.stmts[index], scopes);
                index += 1;
                continue;
            }

            if self.try_shrink_borrow_alias(block, index, scopes) {
                index += 1;
                continue;
            }

            self.lower_stmt(&mut block.stmts[index], scopes);
            index += 1;
        }
    }

    fn try_materialize_iterator_mutation(
        &mut self,
        block: &mut syn::Block,
        index: usize,
        _scopes: &mut ScopeStack,
    ) -> bool {
        let Some(syn::Stmt::Expr(syn::Expr::ForLoop(for_loop), _)) = block.stmts.get(index) else {
            return false;
        };
        let Some(source_ident) = iterator_source_ident(for_loop.expr.as_ref()) else {
            return false;
        };
        if !block_mutates_binding(&for_loop.body, &source_ident) {
            return false;
        }
        self.record_iterator_materialization_note(&source_ident);

        let snapshot_ident =
            quote::format_ident!("__kobo_iter_snapshot_{}", self.iter_snapshot_counter);
        self.iter_snapshot_counter += 1;
        let source_expr = (*for_loop.expr).clone();
        let snapshot_stmt: syn::Stmt = parse_quote! {
            let #snapshot_ident = #source_expr.cloned().collect::<Vec<_>>();
        };
        let mut materialized_loop = for_loop.clone();
        materialized_loop.expr = Box::new(parse_quote!(#snapshot_ident));
        let loop_stmt = syn::Stmt::Expr(syn::Expr::ForLoop(materialized_loop), None);
        let materialized_block = syn::Expr::Block(syn::ExprBlock {
            attrs: Vec::new(),
            label: None,
            block: syn::Block {
                brace_token: syn::token::Brace::default(),
                stmts: vec![snapshot_stmt, loop_stmt],
            },
        });
        block.stmts[index] = syn::Stmt::Expr(materialized_block, None);
        true
    }

    fn record_iterator_materialization_note(&mut self, source_ident: &syn::Ident) {
        let Some(binding) = self
            .ast
            .iter_bindings()
            .find(|binding| binding.ident == *source_ident)
        else {
            return;
        };
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        if self.annotation_notes.iter().any(|note| {
            note.node == node && note.reason.starts_with("iterator-materialization-debt")
        }) {
            return;
        }
        let (kobo_line, _) = self.ast.line_col(binding.span);
        self.annotation_notes.push(AnnotationNote {
            node,
            binding_name: binding.ident.to_string(),
            kobo_line,
            reason: "iterator-materialization-debt: materialized iter() snapshot before mutation"
                .to_owned(),
        });
    }

    pub(super) fn lower_nested_block(&mut self, block: &mut syn::Block, scopes: &mut ScopeStack) {
        scopes.push();
        self.lower_block_statements(block, scopes);
        scopes.pop();
    }

    fn lower_stmt(&mut self, stmt: &mut syn::Stmt, scopes: &mut ScopeStack) {
        match stmt {
            syn::Stmt::Local(local) => self.lower_local(local, scopes),
            syn::Stmt::Item(item) => self.lower_item(item),
            syn::Stmt::Expr(expr, semi) => {
                if let syn::Expr::ForLoop(for_loop) = expr {
                    let policy = parallel::policy_value(&for_loop.attrs)
                        .unwrap_or_else(|| "outside".to_owned());
                    match parallel::lower_for_loop(for_loop) {
                        parallel::ParallelLowering::Parallel => {
                            self.needs_rayon = true;
                        }
                        parallel::ParallelLowering::SerialPolicy => {
                            parallel::mark_serial_policy(for_loop);
                        }
                        parallel::ParallelLowering::BoundaryPolicy => {
                            parallel::mark_boundary_policy(for_loop, &policy);
                        }
                        parallel::ParallelLowering::None => {}
                    }
                }
                if semi.is_none() && self.lower_owned_value_expr(expr, scopes) {
                    return;
                }
                self.lower_expr(expr, scopes);
            }
            syn::Stmt::Macro(stmt_macro) => {
                // Check for spawn block marker macro — wire clone injection.
                // S-53: Determine spawn strategy based on captured bindings' ownership tiers.
                if !spawn::is_any_spawn_block_macro(&stmt_macro.mac) {
                    self.lower_macro_tokens(&mut stmt_macro.mac.tokens, scopes);
                    return;
                }

                let captured = collect_spawn_captures(&stmt_macro.mac.tokens, scopes);
                let use_spawn_local = spawn::is_spawn_local_block_macro(&stmt_macro.mac)
                    || self.any_captured_non_send(&captured);
                if use_spawn_local {
                    self.needs_local_set = true;
                }
                if let Some(spawn_expr) =
                    spawn::lower_spawn_macro_with_strategy(&stmt_macro.mac, use_spawn_local)
                {
                    let semi = stmt_macro.semi_token;
                    if captured.is_empty() {
                        *stmt = syn::Stmt::Expr(spawn_expr, semi);
                    } else {
                        // Inject clone stmts before the spawn, wrap in a block.
                        let mut block_stmts: Vec<syn::Stmt> = Vec::new();
                        for cap in &captured {
                            let action = clone_inject::capture_action(cap);
                            if action == clone_inject::CaptureAction::Clone {
                                let clone_name = clone_inject::clone_var_name(&cap.name);
                                let clone_ident =
                                    syn::Ident::new(&clone_name, proc_macro2::Span::call_site());
                                let orig_ident =
                                    syn::Ident::new(&cap.name, proc_macro2::Span::call_site());
                                let clone_stmt: syn::Stmt = parse_quote! {
                                    let #clone_ident = #orig_ident.clone();
                                };
                                block_stmts.push(clone_stmt);
                            }
                        }
                        block_stmts.push(syn::Stmt::Expr(spawn_expr, semi));
                        let block: syn::Expr = syn::Expr::Block(syn::ExprBlock {
                            attrs: Vec::new(),
                            label: None,
                            block: syn::Block {
                                brace_token: syn::token::Brace::default(),
                                stmts: block_stmts,
                            },
                        });
                        *stmt = syn::Stmt::Expr(block, None);
                    }
                    return;
                }
            }
        }
    }

    fn try_shrink_borrow_alias(
        &mut self,
        block: &mut syn::Block,
        index: usize,
        scopes: &mut ScopeStack,
    ) -> bool {
        let Some(syn::Stmt::Local(local)) = block.stmts.get(index) else {
            return false;
        };
        let Some(alias) = simple_borrow_alias(local, scopes) else {
            return false;
        };
        let Some(next_stmt) = block.stmts.get(index + 1) else {
            self.record_borrow_scope_conservative(local);
            return false;
        };
        if has_later_alias_use(&block.stmts[index + 2..], &alias.alias_ident) {
            self.record_borrow_scope_conservative(local);
            return false;
        }

        let Some((mut method_call, had_semi)) =
            rewritable_method_call(next_stmt, &alias.alias_ident)
        else {
            self.record_borrow_scope_conservative(local);
            return false;
        };

        for argument in &mut method_call.args {
            self.lower_expr(argument, scopes);
        }

        let source_ident = alias.source_ident;
        method_call.receiver = if alias.is_mutable {
            Box::new(parse_quote!(#source_ident.borrow_mut()))
        } else {
            Box::new(parse_quote!(#source_ident.borrow()))
        };

        let replacement = util::build_borrow_scope_block_stmt(method_call, had_semi);
        block.stmts[index] = replacement;
        block.stmts.remove(index + 1);
        true
    }

    fn record_borrow_scope_conservative(&mut self, local: &syn::Local) {
        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            return;
        };
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        let (kobo_line, _) = self.ast.line_col(binding.span);
        if self
            .annotation_notes
            .iter()
            .any(|note| note.node == node && note.reason == "borrow-scope-conservative")
        {
            return;
        }

        self.annotation_notes.push(AnnotationNote {
            node,
            binding_name: binding.ident.to_string(),
            kobo_line,
            reason: "borrow-scope-conservative".to_owned(),
        });
    }

    pub(super) fn record_binding_anchor(
        &mut self,
        binding: &kobo_parser::KoboBinding,
        kind: LoweringAnchorKind,
    ) {
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        self.anchors.push(LoweringAnchor { node, kind });
    }

    fn record_item_anchor(&mut self, ident: &syn::Ident, kind: LoweringAnchorKind) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span) else {
            return;
        };
        self.record_binding_anchor(binding, kind);
    }
}

fn executor_main_attr(
    choice: crate::executor::ExecutorChoice,
    is_main: bool,
) -> Option<syn::Attribute> {
    match executor_attribute(choice, is_main) {
        Some("#[tokio::main]") => Some(parse_quote!(#[tokio::main])),
        Some("#[async_std::main]") => Some(parse_quote!(#[async_std::main])),
        Some(other) => panic!("unsupported executor attribute: {other}"),
        None => None,
    }
}

fn is_executor_main_attr(attr: &syn::Attribute) -> bool {
    let segments: Vec<_> = attr
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    matches!(segments.as_slice(), [executor, main] if main == "main" && (executor == "tokio" || executor == "async_std"))
}

/// Collect bindings captured by a spawn block's token stream.
///
/// Scans tokens for identifiers and checks each against the scope stack.
/// Bindings found in scope are returned as `CapturedBinding` for clone analysis.
/// Conservative: assumes all in-scope bindings referenced in the body are used after spawn.
fn collect_spawn_captures(
    tokens: &proc_macro2::TokenStream,
    scopes: &ScopeStack,
) -> Vec<clone_inject::CapturedBinding> {
    use kobo_ir::OwnershipTier;
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut captures = Vec::new();
    collect_idents_from_tokens(tokens, &mut seen);

    for name in seen {
        let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
        if let Some(tier) = scopes.lookup(&ident) {
            let is_copy = matches!(tier, OwnershipTier::PlainOwned);
            captures.push(clone_inject::CapturedBinding {
                name,
                tier,
                is_copy,
                used_after_spawn: true, // conservative assumption
            });
        }
    }
    captures
}

fn captured_bindings_need_spawn_local(captured: &[clone_inject::CapturedBinding]) -> bool {
    captured.iter().any(|binding| {
        matches!(
            binding.tier,
            kobo_ir::OwnershipTier::RcShared | kobo_ir::OwnershipTier::RcMutShared
        )
    })
}

/// Recursively collect all identifiers from a token stream.
fn collect_idents_from_tokens(
    tokens: &proc_macro2::TokenStream,
    out: &mut std::collections::HashSet<String>,
) {
    for token in tokens.clone() {
        match token {
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                // Skip Rust keywords.
                if !is_rust_keyword(&name) {
                    out.insert(name);
                }
            }
            proc_macro2::TokenTree::Group(group) => {
                collect_idents_from_tokens(&group.stream(), out);
            }
            _ => {}
        }
    }
}

fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "yield"
    )
}

fn ident_from_kobo(name: &str) -> Option<syn::Ident> {
    if is_rust_keyword(name) {
        return None;
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
        return None;
    }
    Some(syn::Ident::new(name, proc_macro2::Span::call_site()))
}

fn iterator_source_ident(expr: &syn::Expr) -> Option<syn::Ident> {
    let syn::Expr::MethodCall(method_call) = expr else {
        return None;
    };
    if method_call.method != "iter" || !method_call.args.is_empty() {
        return None;
    }
    let syn::Expr::Path(path) = method_call.receiver.as_ref() else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    Some(path.path.segments.first()?.ident.clone())
}

fn block_mutates_binding(block: &syn::Block, source_ident: &syn::Ident) -> bool {
    let mut visitor = MutationVisitor {
        source: source_ident.to_string(),
        found: false,
    };
    syn::visit::Visit::visit_block(&mut visitor, block);
    visitor.found
}

struct MutationVisitor {
    source: String,
    found: bool,
}

impl<'ast> syn::visit::Visit<'ast> for MutationVisitor {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if receiver_matches_source(node.receiver.as_ref(), &self.source)
            && mutating_collection_method(&node.method)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if receiver_matches_source(node.left.as_ref(), &self.source) {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_assign(self, node);
    }
}

fn receiver_matches_source(expr: &syn::Expr, source: &str) -> bool {
    match expr {
        syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => path
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == source),
        syn::Expr::Field(field) => receiver_matches_source(field.base.as_ref(), source),
        syn::Expr::Index(index) => receiver_matches_source(index.expr.as_ref(), source),
        syn::Expr::Paren(paren) => receiver_matches_source(paren.expr.as_ref(), source),
        syn::Expr::Group(group) => receiver_matches_source(group.expr.as_ref(), source),
        _ => false,
    }
}

fn mutating_collection_method(method: &syn::Ident) -> bool {
    matches!(
        method.to_string().as_str(),
        "push"
            | "pop"
            | "insert"
            | "remove"
            | "clear"
            | "extend"
            | "retain"
            | "resize"
            | "truncate"
            | "swap_remove"
    )
}

fn wrap_function_body_in_local_set(function: &mut syn::ItemFn) {
    let original_stmts = std::mem::take(&mut function.block.stmts);
    let local_set_stmt: syn::Stmt = parse_quote! {
        let __kobo_local = tokio::task::LocalSet::new();
    };
    let marker_stmt: syn::Stmt = parse_quote! {
        let _ = "kobo: task-local-zone";
    };
    let run_expr: syn::Expr = if function.sig.asyncness.is_some() {
        parse_quote! {
            __kobo_local.run_until(async move { #(#original_stmts)* }).await
        }
    } else {
        parse_quote! {
            tokio::runtime::Handle::current().block_on(__kobo_local.run_until(async move { #(#original_stmts)* }))
        }
    };
    function.block.stmts = vec![local_set_stmt, marker_stmt, syn::Stmt::Expr(run_expr, None)];
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    use syn::parse_quote;

    use super::{
        captured_bindings_need_spawn_local, executor_main_attr, is_executor_main_attr,
        wrap_function_body_in_local_set,
    };
    use crate::executor::ExecutorChoice;
    use crate::lower::rewrite::clone_inject::CapturedBinding;
    use kobo_ir::OwnershipTier;

    fn parsed_executor_attr(choice: ExecutorChoice) -> syn::Attribute {
        executor_main_attr(choice, true).expect("executor attribute should exist")
    }

    #[test]
    fn executor_attr_detector_matches_supported_executors() {
        assert!(is_executor_main_attr(&parsed_executor_attr(
            ExecutorChoice::Tokio
        )));
        assert!(is_executor_main_attr(&parsed_executor_attr(
            ExecutorChoice::AsyncStd
        )));
    }

    #[test]
    fn executor_attr_detector_ignores_other_attributes() {
        let attr: syn::Attribute = parse_quote!(#[allow(dead_code)]);
        assert!(!is_executor_main_attr(&attr));
    }

    #[test]
    fn executor_attr_renders_expected_tokens() {
        let attr = parsed_executor_attr(ExecutorChoice::Tokio);
        assert_eq!(attr.to_token_stream().to_string(), "# [tokio :: main]");
    }

    #[test]
    fn local_set_wrapper_for_async_function_uses_run_until_await() {
        let mut function: syn::ItemFn = parse_quote! {
            async fn run_local() -> u32 {
                tokio::task::spawn_local(async move {});
                7
            }
        };

        wrap_function_body_in_local_set(&mut function);
        let rendered = function.to_token_stream().to_string();

        assert!(rendered.contains("tokio :: task :: LocalSet :: new"));
        assert!(rendered.contains("run_until"));
        assert!(rendered.contains(". await"));
        assert!(rendered.contains("spawn_local"));
        assert!(rendered.contains("7"));
    }

    #[test]
    fn local_set_wrapper_for_sync_function_uses_runtime_handle() {
        let mut function: syn::ItemFn = parse_quote! {
            fn run_local() {
                tokio::task::spawn_local(async move {});
            }
        };

        wrap_function_body_in_local_set(&mut function);
        let rendered = function.to_token_stream().to_string();

        assert!(rendered.contains("tokio :: task :: LocalSet :: new"));
        assert!(rendered.contains("tokio :: runtime :: Handle :: current"));
        assert!(rendered.contains("block_on"));
        assert!(rendered.contains("spawn_local"));
    }

    #[test]
    fn captured_rc_tier_requests_spawn_local() {
        let captured = vec![CapturedBinding {
            name: "world".to_owned(),
            tier: OwnershipTier::RcMutShared,
            is_copy: false,
            used_after_spawn: true,
        }];

        assert!(captured_bindings_need_spawn_local(&captured));
    }

    #[test]
    fn captured_arc_tier_keeps_tokio_spawn() {
        let captured = vec![CapturedBinding {
            name: "state".to_owned(),
            tier: OwnershipTier::ArcMutShared,
            is_copy: false,
            used_after_spawn: true,
        }];

        assert!(!captured_bindings_need_spawn_local(&captured));
    }
}
