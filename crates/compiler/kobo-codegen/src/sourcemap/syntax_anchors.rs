use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use super::proof_anchors::{GeneratedProofEventAnchor, GeneratedProofEventRole};
use super::spans::rs_span_from_syn;

struct GeneratedEventCollector {
    function: String,
    anchors: Vec<GeneratedProofEventAnchor>,
}

pub(super) fn generated_proof_event_anchors(rs_source: &str) -> Vec<GeneratedProofEventAnchor> {
    let Ok(file) = syn::parse_file(rs_source) else {
        return Vec::new();
    };
    let mut anchors = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(function) => {
                anchors.extend(collect_function_event_anchors(
                    function.sig.ident.to_string(),
                    &function.sig.inputs,
                    &function.block,
                    function.sig.ident.span(),
                ));
            }
            syn::Item::Impl(item_impl) => {
                for impl_item in &item_impl.items {
                    let syn::ImplItem::Fn(function) = impl_item else {
                        continue;
                    };
                    anchors.extend(collect_function_event_anchors(
                        function.sig.ident.to_string(),
                        &function.sig.inputs,
                        &function.block,
                        function.sig.ident.span(),
                    ));
                }
            }
            _ => {}
        }
    }
    anchors
}

fn collect_function_event_anchors(
    function: String,
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
    block: &syn::Block,
    function_span: proc_macro2::Span,
) -> Vec<GeneratedProofEventAnchor> {
    let mut collector = GeneratedEventCollector {
        function,
        anchors: Vec::new(),
    };
    collector.push_anchor(
        "return",
        None,
        None,
        GeneratedProofEventRole::ReturnFunction,
        function_span,
    );
    for input in inputs {
        let syn::FnArg::Typed(argument) = input else {
            continue;
        };
        let Some(binding) = binding_ident(&argument.pat) else {
            continue;
        };
        collector.push_anchor(
            "create",
            Some(binding.to_string()),
            None,
            GeneratedProofEventRole::Create,
            binding.span(),
        );
    }
    collector.visit_block(block);
    collector.anchors
}

impl GeneratedEventCollector {
    fn push_anchor(
        &mut self,
        kind: &'static str,
        binding: Option<String>,
        detail: Option<String>,
        role: GeneratedProofEventRole,
        span: proc_macro2::Span,
    ) {
        self.anchors.push(GeneratedProofEventAnchor {
            function: self.function.clone(),
            kind,
            binding,
            detail,
            role,
            rs_span: rs_span_from_syn(span),
        });
    }
}

impl<'ast> Visit<'ast> for GeneratedEventCollector {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(init) = &local.init {
            if let Some(binding) = binding_ident(&local.pat) {
                self.push_anchor(
                    "create",
                    Some(binding.to_string()),
                    None,
                    GeneratedProofEventRole::Create,
                    event_expr_span(init.expr.as_ref()),
                );
                self.push_anchor(
                    "discharge",
                    Some(binding.to_string()),
                    Some("suppressed".to_owned()),
                    GeneratedProofEventRole::Discharge,
                    local.span(),
                );
            }
            if let Some(binding) = expr_path_ident(init.expr.as_ref()) {
                self.push_anchor(
                    "move",
                    Some(binding),
                    None,
                    GeneratedProofEventRole::Move,
                    event_expr_span(init.expr.as_ref()),
                );
            }
        }
        visit::visit_local(self, local);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if let Some(binding) = receiver_ident(call.receiver.as_ref()) {
            self.push_anchor(
                "discharge",
                Some(binding),
                Some(terminal_action_name(&call.method.to_string())),
                GeneratedProofEventRole::Discharge,
                call.span(),
            );
        }
        let method_detail = Some(call.method.to_string());
        self.push_anchor(
            "escape",
            None,
            method_detail.clone(),
            GeneratedProofEventRole::Escape,
            call.span(),
        );
        self.push_anchor(
            "opaque_boundary",
            None,
            method_detail,
            GeneratedProofEventRole::OpaqueBoundary,
            call.span(),
        );
        self.push_anchor(
            "opaque_boundary",
            None,
            None,
            GeneratedProofEventRole::OpaqueBoundary,
            call.span(),
        );
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let call_detail = call_detail(call);
        for argument in &call.args {
            if let Some(binding) = expr_path_ident(argument) {
                self.push_anchor(
                    "discharge",
                    Some(binding.clone()),
                    Some("escape".to_owned()),
                    GeneratedProofEventRole::Discharge,
                    call.span(),
                );
                self.push_anchor(
                    "transfer",
                    Some(binding),
                    call_detail.clone(),
                    GeneratedProofEventRole::Transfer,
                    call.span(),
                );
            }
        }
        self.push_anchor(
            "escape",
            None,
            call_detail.clone(),
            GeneratedProofEventRole::Escape,
            call.span(),
        );
        self.push_anchor(
            "opaque_boundary",
            None,
            call_detail,
            GeneratedProofEventRole::OpaqueBoundary,
            call.span(),
        );
        self.push_anchor(
            "opaque_boundary",
            None,
            None,
            GeneratedProofEventRole::OpaqueBoundary,
            call.span(),
        );
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_try(&mut self, expr_try: &'ast syn::ExprTry) {
        self.push_anchor(
            "error_exit",
            None,
            None,
            GeneratedProofEventRole::ErrorExit,
            event_expr_span(expr_try.expr.as_ref()),
        );
        visit::visit_expr_try(self, expr_try);
    }

    fn visit_expr_return(&mut self, expr_return: &'ast syn::ExprReturn) {
        if let Some(binding) = expr_return.expr.as_deref().and_then(expr_path_ident) {
            self.push_anchor(
                "discharge",
                Some(binding),
                Some("return".to_owned()),
                GeneratedProofEventRole::Discharge,
                expr_return.span(),
            );
        }
        self.push_anchor(
            "return",
            None,
            None,
            GeneratedProofEventRole::ReturnTerminator,
            expr_return.span(),
        );
        visit::visit_expr_return(self, expr_return);
    }

    fn visit_expr_await(&mut self, expr_await: &'ast syn::ExprAwait) {
        self.push_anchor(
            "cancel",
            None,
            None,
            GeneratedProofEventRole::Cancel,
            expr_await.span(),
        );
        visit::visit_expr_await(self, expr_await);
    }

    fn visit_expr_macro(&mut self, expr_macro: &'ast syn::ExprMacro) {
        self.record_macro(&expr_macro.mac);
        visit::visit_expr_macro(self, expr_macro);
    }

    fn visit_stmt_macro(&mut self, stmt_macro: &'ast syn::StmtMacro) {
        self.record_macro(&stmt_macro.mac);
        visit::visit_stmt_macro(self, stmt_macro);
    }
}

impl GeneratedEventCollector {
    fn record_macro(&mut self, mac: &syn::Macro) {
        if path_last_ident(&mac.path).as_deref() == Some("panic") {
            self.push_anchor(
                "panic",
                None,
                None,
                GeneratedProofEventRole::Panic,
                mac.span(),
            );
        }
    }
}

fn terminal_action_name(method: &str) -> String {
    match method {
        "detach_with_policy" => "detach-with-policy".to_owned(),
        other => other.to_owned(),
    }
}

fn call_detail(call: &syn::ExprCall) -> Option<String> {
    match call.func.as_ref() {
        syn::Expr::Path(path) => path_last_ident(&path.path),
        _ => None,
    }
}

fn event_expr_span(expr: &syn::Expr) -> proc_macro2::Span {
    first_call_like_span(expr).unwrap_or_else(|| expr.span())
}

fn first_call_like_span(expr: &syn::Expr) -> Option<proc_macro2::Span> {
    match expr {
        syn::Expr::Call(call) => Some(call.span()),
        syn::Expr::MethodCall(call) => Some(call.span()),
        syn::Expr::Block(block) => block
            .block
            .stmts
            .iter()
            .find_map(first_call_like_span_from_stmt),
        syn::Expr::Paren(paren) => first_call_like_span(paren.expr.as_ref()),
        syn::Expr::Try(expr_try) => first_call_like_span(expr_try.expr.as_ref()),
        syn::Expr::Await(await_expr) => first_call_like_span(await_expr.base.as_ref()),
        _ => None,
    }
}

fn first_call_like_span_from_stmt(stmt: &syn::Stmt) -> Option<proc_macro2::Span> {
    match stmt {
        syn::Stmt::Expr(expr, _) => first_call_like_span(expr),
        syn::Stmt::Local(local) => local
            .init
            .as_ref()
            .and_then(|init| first_call_like_span(init.expr.as_ref())),
        _ => None,
    }
}

fn binding_ident(pattern: &syn::Pat) -> Option<&syn::Ident> {
    match pattern {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}

fn expr_path_ident(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) => path_last_ident(&path.path),
        syn::Expr::Paren(paren) => expr_path_ident(paren.expr.as_ref()),
        _ => None,
    }
}

fn receiver_ident(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) => path_last_ident(&path.path),
        syn::Expr::Paren(paren) => receiver_ident(paren.expr.as_ref()),
        _ => None,
    }
}

fn path_last_ident(path: &syn::Path) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}
