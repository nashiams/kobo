use super::super::binding::{apply_tier_to_fn_arg_type, binding_for_pat, fn_arg_lowering_tier};
use super::super::scope::{type_name_from_syn, ScopeStack};
use super::super::strict::StrictGuardCounter;
use super::attrs::{executor_main_attr, is_executor_main_attr};
use super::{split_borrow, syntax_support, Lowerer, LoweringAnchorKind};
use crate::lower::strict;
use syn::parse_quote;

impl<'a> Lowerer<'a> {
    pub(crate) fn lower_items(&mut self, items: &mut [syn::Item]) {
        for item in items {
            self.lower_item(item);
        }
    }

    pub(super) fn lower_item(&mut self, item: &mut syn::Item) {
        match item {
            syn::Item::Fn(function) => {
                self.apply_executor_attribute(function);
                // Check if this is an @strict fn by span matching against ast.strict_fns().
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
                    let ts = strict::lower_strict_fn(
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
                syntax_support::strip_kobo_attrs(&mut item_type.attrs);
            }
            syn::Item::Enum(item_enum) => {
                syntax_support::strip_kobo_attrs(&mut item_enum.attrs);
            }
            _ => {}
        }
    }

    fn lower_struct_item(&mut self, item_struct: &mut syn::ItemStruct) {
        syntax_support::strip_kobo_attrs(&mut item_struct.attrs);
        for field in &mut item_struct.fields {
            if syntax_support::has_kobo_attr(&field.attrs, "counter") {
                field.ty = parse_quote!(std::sync::atomic::AtomicU64);
            } else if syntax_support::has_kobo_attr(&field.attrs, "live") {
                self.concurrent_support.live_cell = true;
                let original_ty = field.ty.clone();
                field.ty = parse_quote!(KoboArcSwap<#original_ty>);
            } else if syntax_support::has_kobo_attr(&field.attrs, "view_distance") {
                self.concurrent_support.view_distance = true;
                field.ty = parse_quote!(KoboViewDistance);
            }
            syntax_support::strip_kobo_attrs(&mut field.attrs);
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
        syntax_support::strip_kobo_attrs(&mut item_impl.attrs);

        // Detect split-borrow sites and apply destructuring.
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
        syntax_support::strip_kobo_attrs(&mut method.attrs);
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
    }
}
