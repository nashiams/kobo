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

use super::binding::{apply_tier_to_fn_arg_type, binding_for_pat};
use super::borrow_scope::{has_later_alias_use, rewritable_method_call, simple_borrow_alias};
use super::plan::{AnnotationNote, LoweringPlan};
use super::scope::{ScopeStack, type_name_from_syn};
use super::strict::StrictGuardCounter;
use super::{LoweringAnchor, LoweringAnchorKind};
use crate::CodegenOptions;
use crate::executor::executor_attribute;

pub(crate) struct Lowerer<'a> {
    pub(super) ast: &'a KoboFile,
    pub(super) plan: &'a LoweringPlan,
    pub(super) kir: &'a kobo_ir::Kir,
    pub(super) options: &'a CodegenOptions,
    pub(super) in_async_context: bool,
    pub(super) strict_counter: StrictGuardCounter,
    pub(super) annotation_notes: Vec<AnnotationNote>,
    pub(super) anchors: Vec<LoweringAnchor>,
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
            strict_counter: StrictGuardCounter::new(),
            annotation_notes: Vec::new(),
            anchors: Vec::new(),
        }
    }

    pub(crate) fn lower_items(&mut self, items: &mut [syn::Item]) {
        for item in items {
            self.lower_item(item);
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<AnnotationNote>, Vec<LoweringAnchor>) {
        (self.annotation_notes, self.anchors)
    }

    fn lower_item(&mut self, item: &mut syn::Item) {
        match item {
            syn::Item::Fn(function) => {
                self.apply_executor_attribute(function);
                // P5: check if this is an @strict fn (by span matching against ast.strict_fns()).
                use syn::spanned::Spanned;
                let fn_span = self.ast.span_from_syn(function.span());
                if let Some(kfn) = self.ast.strict_fns().iter().find(|f| f.span == fn_span).cloned() {
                    let fn_body_span = self.ast.span_from_syn(function.block.span());
                    let cap = self.kir.strict_capture_sets().iter()
                        .find(|cs| cs.block_span == fn_body_span)
                        .cloned();
                    let mode = self.kir.strict_fn_modes().get(&kfn.span)
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
                util::strip_kobo_attrs(&mut item_struct.attrs);
                for field in &mut item_struct.fields {
                    util::strip_kobo_attrs(&mut field.attrs);
                }
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
        let items_snapshot: Vec<syn::Item> =
            vec![syn::Item::Impl(item_impl.clone())];
        let split_sites =
            kobo_analysis::split_borrow::detect_split_borrow_sites(&items_snapshot);

        for impl_item in &mut item_impl.items {
            if let syn::ImplItem::Fn(method) = impl_item {
                // Apply split-borrow rewrite if a site was detected for this method.
                if let Some(site) = split_sites
                    .iter()
                    .find(|s| s.method_name == method.sig.ident.to_string())
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
                    let tier = self.plan.tier_for_binding(binding);
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
        util::strip_kobo_attrs(&mut function.attrs);
        // Reset guard counter per function (Contract C08 / Trap 16).
        self.strict_counter = StrictGuardCounter::new();
        let prior_async_context = self.in_async_context;
        self.in_async_context = function.sig.asyncness.is_some();
        let mut scopes = ScopeStack::new();
        scopes.push();
        self.lower_function_params(&mut function.sig.inputs, &mut scopes);
        self.lower_block_statements(&mut function.block, &mut scopes);
        scopes.pop();
        self.in_async_context = prior_async_context;
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

            let tier = self.plan.tier_for_binding(binding);
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
            if self.try_shrink_borrow_alias(block, index, scopes) {
                index += 1;
                continue;
            }

            self.lower_stmt(&mut block.stmts[index], scopes);
            index += 1;
        }
    }

    pub(super) fn lower_nested_block(
        &mut self,
        block: &mut syn::Block,
        scopes: &mut ScopeStack,
    ) {
        scopes.push();
        self.lower_block_statements(block, scopes);
        scopes.pop();
    }

    fn lower_stmt(&mut self, stmt: &mut syn::Stmt, scopes: &mut ScopeStack) {
        match stmt {
            syn::Stmt::Local(local) => self.lower_local(local, scopes),
            syn::Stmt::Item(item) => self.lower_item(item),
            syn::Stmt::Expr(expr, _) => self.lower_expr(expr, scopes),
            syn::Stmt::Macro(stmt_macro) => {
                // Check for spawn block marker macro.
                if let Some(replacement) = spawn::lower_spawn_macro(&stmt_macro.mac) {
                    let semi = stmt_macro.semi_token;
                    *stmt = syn::Stmt::Expr(replacement, semi);
                    return;
                }
                self.lower_macro_tokens(&mut stmt_macro.mac.tokens, scopes);
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

#[cfg(test)]
mod tests {
    use quote::ToTokens;
    use syn::parse_quote;

    use super::{executor_main_attr, is_executor_main_attr};
    use crate::executor::ExecutorChoice;

    fn parsed_executor_attr(choice: ExecutorChoice) -> syn::Attribute {
        executor_main_attr(choice, true).expect("executor attribute should exist")
    }

    #[test]
    fn executor_attr_detector_matches_supported_executors() {
        assert!(is_executor_main_attr(&parsed_executor_attr(ExecutorChoice::Tokio)));
        assert!(is_executor_main_attr(&parsed_executor_attr(ExecutorChoice::AsyncStd)));
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
}
