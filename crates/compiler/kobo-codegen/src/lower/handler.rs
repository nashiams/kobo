use quote::{quote, ToTokens};
use syn::spanned::Spanned;

use crate::HandlerLifecycleEvidence;

#[derive(Clone)]
struct HandlerSpec {
    name: String,
    source_line: usize,
    cleanup_hook: Option<syn::Path>,
    terminal_actions: Vec<String>,
}

impl HandlerSpec {
    fn cleanup_name(&self) -> Option<String> {
        self.cleanup_hook
            .as_ref()
            .and_then(|path| path.segments.last())
            .map(|segment| segment.ident.to_string())
    }
}

pub(crate) struct HandlerSupport {
    pub(crate) items: Vec<syn::Item>,
    pub(crate) evidence: Vec<HandlerLifecycleEvidence>,
}

pub(crate) fn handler_support(ast: &kobo_parser::KoboFile, file: &syn::File) -> HandlerSupport {
    let specs = handler_specs(ast, file);
    if specs.is_empty() {
        return HandlerSupport {
            items: Vec::new(),
            evidence: Vec::new(),
        };
    }

    HandlerSupport {
        items: vec![
            handler_cleanup_future_item(),
            handler_cleanup_runtime_item(),
            handler_cleanup_runtime_impl_item(),
            handler_outcome_item(),
            handler_metrics_struct_item(),
            handler_guard_struct_item(),
            handler_outcome_impl_item(),
            handler_guard_impl_item(),
            handler_guard_drop_impl_item(),
        ],
        evidence: specs.iter().map(handler_evidence).collect(),
    }
}

pub(crate) fn append_handler_support_items(file: &mut syn::File, items: Vec<syn::Item>) {
    file.items.extend(items);
}

pub(crate) fn lower_item_function(ast: &kobo_parser::KoboFile, function: &mut syn::ItemFn) -> bool {
    let Some(spec) = handler_spec(ast, &function.attrs, &function.sig) else {
        return false;
    };
    let source_doc = handler_source_doc(&spec.name, spec.source_line);
    function.attrs.push(syn::parse_quote!(#[doc = #source_doc]));
    let original_stmts = std::mem::take(&mut function.block.stmts);
    function.block.stmts = handler_body_stmts(&function.sig, original_stmts, &spec);
    true
}

fn handler_specs(ast: &kobo_parser::KoboFile, file: &syn::File) -> Vec<HandlerSpec> {
    file.items
        .iter()
        .filter_map(|item| {
            let syn::Item::Fn(function) = item else {
                return None;
            };
            handler_spec(ast, &function.attrs, &function.sig).map(|mut spec| {
                spec.terminal_actions = terminal_actions_from_block(&function.block);
                spec
            })
        })
        .collect()
}

fn handler_spec(
    ast: &kobo_parser::KoboFile,
    attrs: &[syn::Attribute],
    signature: &syn::Signature,
) -> Option<HandlerSpec> {
    let handler_attr = attrs.iter().find(|attr| is_kobo_attr(attr, "handler"))?;
    Some(HandlerSpec {
        name: signature.ident.to_string(),
        source_line: source_line(ast, handler_attr),
        cleanup_hook: attrs.iter().find_map(cleanup_hook_path),
        terminal_actions: Vec::new(),
    })
}

fn is_kobo_attr(attr: &syn::Attribute, name: &str) -> bool {
    let segments = attr.path().segments.iter().collect::<Vec<_>>();
    segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == name
}

fn cleanup_hook_path(attr: &syn::Attribute) -> Option<syn::Path> {
    if !is_kobo_attr(attr, "cleanup") {
        return None;
    }
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    syn::parse2::<syn::Path>(list.tokens.clone()).ok()
}

fn source_line(ast: &kobo_parser::KoboFile, attr: &syn::Attribute) -> usize {
    let span = ast.span_from_syn(attr.span());
    let (line, _) = ast.line_col(span);
    line
}

fn handler_body_stmts(
    signature: &syn::Signature,
    original_stmts: Vec<syn::Stmt>,
    spec: &HandlerSpec,
) -> Vec<syn::Stmt> {
    let mut stmts = clone_arc_params(signature);
    if returns_result(signature) {
        stmts.extend(result_handler_body(original_stmts, spec));
    } else {
        stmts.extend(value_handler_body(original_stmts, spec));
    }
    stmts
}

fn clone_arc_params(signature: &syn::Signature) -> Vec<syn::Stmt> {
    signature
        .inputs
        .iter()
        .filter_map(|input| {
            let syn::FnArg::Typed(argument) = input else {
                return None;
            };
            if !quote!(#argument.ty).to_string().contains("Arc") {
                return None;
            }
            let syn::Pat::Ident(pat_ident) = argument.pat.as_ref() else {
                return None;
            };
            let ident = &pat_ident.ident;
            Some(syn::parse_quote! {
                let #ident = #ident.clone();
            })
        })
        .collect()
}

fn returns_result(signature: &syn::Signature) -> bool {
    let syn::ReturnType::Type(_, ty) = &signature.output else {
        return false;
    };
    let syn::Type::Path(type_path) = ty.as_ref() else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Result")
}

fn result_handler_body(original_stmts: Vec<syn::Stmt>, spec: &HandlerSpec) -> Vec<syn::Stmt> {
    let handler_name = spec.name.as_str();
    let source_line = spec.source_line;
    let cleanup_registration = cleanup_registration_stmt(spec);
    syn::parse_quote! {
        let mut __kobo_handler_guard = KoboHandlerLifecycleGuard::enter(#handler_name, #source_line);
        #cleanup_registration
        let __kobo_handler_result = async move {
            #(#original_stmts)*
        }.await;
        match __kobo_handler_result {
            Ok(__kobo_handler_reply) => {
                __kobo_handler_guard.run_registered_cleanup("success").await;
                __kobo_handler_guard.record_reply();
                __kobo_handler_guard.metrics_boundary(#handler_name);
                let __kobo_handler_outcome: KoboHandlerOutcome<_, _> =
                    KoboHandlerOutcome::reply(__kobo_handler_reply);
                match __kobo_handler_outcome {
                    KoboHandlerOutcome::Reply(__kobo_handler_value) => Ok(__kobo_handler_value),
                    KoboHandlerOutcome::Reject(__kobo_handler_error) => Err(__kobo_handler_error),
                    KoboHandlerOutcome::Cancelled => unreachable!("handler reply outcome cannot be cancelled after success"),
                }
            }
            Err(__kobo_handler_error) => {
                __kobo_handler_guard.run_registered_cleanup("error").await;
                __kobo_handler_guard.record_reject();
                __kobo_handler_guard.metrics_boundary(#handler_name);
                let __kobo_handler_outcome: KoboHandlerOutcome<_, _> =
                    KoboHandlerOutcome::reject(__kobo_handler_error);
                match __kobo_handler_outcome {
                    KoboHandlerOutcome::Reply(__kobo_handler_value) => Ok(__kobo_handler_value),
                    KoboHandlerOutcome::Reject(__kobo_handler_error) => Err(__kobo_handler_error),
                    KoboHandlerOutcome::Cancelled => unreachable!("handler reject outcome cannot be cancelled after error"),
                }
            }
        }
    }
}

fn value_handler_body(original_stmts: Vec<syn::Stmt>, spec: &HandlerSpec) -> Vec<syn::Stmt> {
    let handler_name = spec.name.as_str();
    let source_line = spec.source_line;
    let cleanup_registration = cleanup_registration_stmt(spec);
    syn::parse_quote! {
        let mut __kobo_handler_guard = KoboHandlerLifecycleGuard::enter(#handler_name, #source_line);
        #cleanup_registration
        let __kobo_handler_value = async move {
            #(#original_stmts)*
        }.await;
        __kobo_handler_guard.run_registered_cleanup("success").await;
        __kobo_handler_guard.record_reply();
        __kobo_handler_guard.metrics_boundary(#handler_name);
        __kobo_handler_value
    }
}

fn cleanup_registration_stmt(spec: &HandlerSpec) -> syn::Stmt {
    match (spec.cleanup_name(), spec.cleanup_hook.as_ref()) {
        (Some(name), Some(path)) => syn::parse_quote! {
            __kobo_handler_guard.register_cleanup(#name, || -> KoboHandlerCleanupFuture {
                Box::pin(#path())
            });
        },
        _ => syn::parse_quote! {
            __kobo_handler_guard.register_no_cleanup();
        },
    }
}

fn handler_source_doc(handler_name: &str, source_line: usize) -> String {
    format!("kobo: handler {handler_name} source_line={source_line}")
}

fn handler_cleanup_future_item() -> syn::Item {
    syn::parse_quote! {
        type KoboHandlerCleanupFuture =
            std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
    }
}

fn handler_cleanup_runtime_item() -> syn::Item {
    syn::parse_quote! {
        struct KoboHandlerCleanupRuntime;
    }
}

fn handler_cleanup_runtime_impl_item() -> syn::Item {
    syn::parse_quote! {
        impl KoboHandlerCleanupRuntime {
            fn run(cleanup: fn() -> KoboHandlerCleanupFuture) {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(cleanup());
                } else {
                    let _ = cleanup;
                }
            }
        }
    }
}

fn handler_outcome_item() -> syn::Item {
    syn::parse_quote! {
        enum KoboHandlerOutcome<TReply, TError> {
            Reply(TReply),
            Reject(TError),
            Cancelled,
        }
    }
}

fn handler_guard_struct_item() -> syn::Item {
    syn::parse_quote! {
        #[derive(Default)]
        struct KoboHandlerLifecycleMetrics {
            entries: u64,
            exits: u64,
            replies: u64,
            rejects: u64,
            cancels: u64,
            cleanup_runs: u64,
        }
    }
}

fn handler_metrics_struct_item() -> syn::Item {
    syn::parse_quote! {
        struct KoboHandlerLifecycleGuard {
            handler: &'static str,
            source_line: usize,
            cleanup_hook: Option<&'static str>,
            cleanup: Option<fn() -> KoboHandlerCleanupFuture>,
            state: &'static str,
            metrics: KoboHandlerLifecycleMetrics,
            cleanup_status: &'static str,
            tracing_span_closed: bool,
        }
    }
}

fn handler_outcome_impl_item() -> syn::Item {
    syn::parse_quote! {
        impl<TReply, TError> KoboHandlerOutcome<TReply, TError> {
            fn reply(value: TReply) -> Self {
                Self::Reply(value)
            }

            fn reject(error: TError) -> Self {
                Self::Reject(error)
            }

            fn cancel() -> Self {
                Self::Cancelled
            }
        }
    }
}

fn handler_guard_impl_item() -> syn::Item {
    syn::parse_quote! {
        impl KoboHandlerLifecycleGuard {
            fn enter(handler: &'static str, source_line: usize) -> Self {
                Self {
                    handler,
                    source_line,
                    cleanup_hook: None,
                    cleanup: None,
                    state: "entered",
                    metrics: KoboHandlerLifecycleMetrics {
                        entries: 1,
                        ..KoboHandlerLifecycleMetrics::default()
                    },
                    cleanup_status: "pending",
                    tracing_span_closed: false,
                }
            }

            fn register_cleanup(
                &mut self,
                cleanup_hook: &'static str,
                cleanup: fn() -> KoboHandlerCleanupFuture,
            ) {
                self.cleanup_hook = Some(cleanup_hook);
                self.cleanup = Some(cleanup);
            }

            fn register_no_cleanup(&mut self) {
                self.cleanup_hook = None;
                self.cleanup = None;
                self.cleanup_status = "not-registered";
            }

            fn note_no_cleanup_hook(&mut self) {
                self.cleanup_status = "not-registered";
            }

            async fn run_registered_cleanup(&mut self, reason: &'static str) {
                if let Some(cleanup) = self.cleanup.take() {
                    self.record_cleanup_run(reason);
                    cleanup().await;
                } else {
                    self.note_no_cleanup_hook();
                }
            }

            fn record_cleanup_run(&mut self, reason: &'static str) {
                self.cleanup_status = reason;
                self.metrics.cleanup_runs += 1;
            }

            fn record_reply(&mut self) {
                self.state = "reply";
                self.metrics.replies += 1;
            }

            fn record_reject(&mut self) {
                self.state = "reject";
                self.metrics.rejects += 1;
            }

            fn record_cancel(&mut self) {
                self.state = "cancel";
                self.metrics.cancels += 1;
            }

            fn metrics_boundary(&mut self, handler: &'static str) {
                self.metrics.exits += 1;
                let _ = (
                    "handler-metrics-boundary",
                    handler,
                    self.metrics.entries,
                    self.metrics.exits,
                    self.metrics.replies,
                    self.metrics.rejects,
                    self.metrics.cancels,
                    self.metrics.cleanup_runs,
                );
            }

            fn close_tracing_span(&mut self) {
                self.tracing_span_closed = true;
                let _ = (
                    "handler-tracing-close",
                    self.handler,
                    self.source_line,
                    self.state,
                    self.cleanup_status,
                    self.tracing_span_closed,
                );
            }

            fn run_cancel_cleanup_on_drop(&mut self) {
                if self.state == "entered" {
                    if let Some(cleanup) = self.cleanup.take() {
                        self.record_cleanup_run("cancel-drop");
                        KoboHandlerCleanupRuntime::run(cleanup);
                    }
                    self.record_cancel();
                }
            }
        }
    }
}

fn terminal_actions_from_block(block: &syn::Block) -> Vec<String> {
    let rendered = block.to_token_stream().to_string();
    ["reply", "reject", "cancel"]
        .into_iter()
        .filter(|action| {
            rendered.contains(&format!(". {action} (")) || rendered.contains(&format!(".{action}("))
        })
        .map(str::to_owned)
        .collect()
}

fn handler_evidence(spec: &HandlerSpec) -> HandlerLifecycleEvidence {
    HandlerLifecycleEvidence {
        name: spec.name.clone(),
        source_line: spec.source_line,
        cleanup_hook: spec.cleanup_name(),
        terminal_actions: spec.terminal_actions.clone(),
        tracing_boundary: "drop-closes-span".to_owned(),
        metrics_boundary: "handler-entry-exit-counters".to_owned(),
        cleanup_boundary: "registered-success-error-cancel".to_owned(),
        cancel_cleanup: "drop-spawns-registered-cleanup".to_owned(),
    }
}

fn handler_guard_drop_impl_item() -> syn::Item {
    syn::parse_quote! {
        impl Drop for KoboHandlerLifecycleGuard {
            fn drop(&mut self) {
                self.run_cancel_cleanup_on_drop();
                self.close_tracing_span();
            }
        }
    }
}
