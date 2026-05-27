use syn::parse_quote;

use super::super::binding::{apply_tier_to_fn_arg_type, binding_for_pat, fn_arg_lowering_tier};
use super::super::handler;
use super::super::scope::{type_name_from_syn, ScopeStack};
use super::super::strict::StrictGuardCounter;
use super::field_capability::ident_from_kobo;
use super::{syntax_support, tick, Lowerer, LoweringAnchorKind};

impl<'a> Lowerer<'a> {
    pub(super) fn lower_function(&mut self, function: &mut syn::ItemFn) {
        let lowered_handler = handler::lower_item_function(self.ast, function);
        // #[kobo::tick(rate=N)] injects an interval loop before lowering body.
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
        // #[kobo::handler] wraps the body in a per-request isolation boundary.
        let is_handler = !lowered_handler
            && function.attrs.iter().any(|attr| {
                let segments: Vec<_> = attr.path().segments.iter().collect();
                segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "handler"
            });
        if is_handler {
            // Per-request isolation clones Arc params into locals, then wraps in catch_unwind.
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
        syntax_support::strip_kobo_attrs(&mut function.attrs);
        // Reset guard counter per function.
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

    pub(super) fn lower_function_params(
        &mut self,
        inputs: &mut syn::punctuated::Punctuated<syn::FnArg, syn::Token![,]>,
        scopes: &mut ScopeStack,
    ) {
        for input in inputs {
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            // Strip #[kobo::...] attributes from parameters.
            syntax_support::strip_kobo_attrs(&mut argument.attrs);
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

pub(super) fn wrap_function_body_in_local_set(function: &mut syn::ItemFn) {
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
