use quote::quote;
use syn::spanned::Spanned;

#[derive(Clone)]
struct HandlerSpec {
    name: String,
    source_line: usize,
    cleanup_hook: Option<syn::Path>,
}

impl HandlerSpec {
    fn cleanup_name(&self) -> Option<String> {
        self.cleanup_hook
            .as_ref()
            .and_then(|path| path.segments.last())
            .map(|segment| segment.ident.to_string())
    }
}

pub(crate) fn handler_support_items(file: &syn::File) -> Vec<syn::Item> {
    if !has_handler(file) {
        return Vec::new();
    }

    vec![
        handler_outcome_item(),
        handler_guard_struct_item(),
        handler_outcome_impl_item(),
        handler_guard_impl_item(),
        handler_guard_drop_impl_item(),
    ]
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

fn has_handler(file: &syn::File) -> bool {
    file.items.iter().any(item_has_handler)
}

fn item_has_handler(item: &syn::Item) -> bool {
    match item {
        syn::Item::Fn(function) => attrs_have_handler(&function.attrs),
        syn::Item::Impl(item_impl) => item_impl.items.iter().any(|item| match item {
            syn::ImplItem::Fn(method) => attrs_have_handler(&method.attrs),
            _ => false,
        }),
        _ => false,
    }
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
    })
}

fn attrs_have_handler(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| is_kobo_attr(attr, "handler"))
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
    let cleanup_name = spec.cleanup_name();
    let cleanup_registration = cleanup_registration_stmt(cleanup_name.as_deref());
    let success_cleanup = cleanup_call_stmt(spec.cleanup_hook.as_ref());
    let error_cleanup = cleanup_call_stmt(spec.cleanup_hook.as_ref());
    syn::parse_quote! {
        let mut __kobo_handler_guard = KoboHandlerLifecycleGuard::enter(#handler_name, #source_line);
        #cleanup_registration
        let __kobo_handler_result = async move {
            #(#original_stmts)*
        }.await;
        match __kobo_handler_result {
            Ok(__kobo_handler_reply) => {
                #success_cleanup
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
                #error_cleanup
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
    let cleanup_name = spec.cleanup_name();
    let cleanup_registration = cleanup_registration_stmt(cleanup_name.as_deref());
    let success_cleanup = cleanup_call_stmt(spec.cleanup_hook.as_ref());
    syn::parse_quote! {
        let mut __kobo_handler_guard = KoboHandlerLifecycleGuard::enter(#handler_name, #source_line);
        #cleanup_registration
        let __kobo_handler_value = async move {
            #(#original_stmts)*
        }.await;
        #success_cleanup
        __kobo_handler_guard.record_reply();
        __kobo_handler_guard.metrics_boundary(#handler_name);
        __kobo_handler_value
    }
}

fn cleanup_registration_stmt(cleanup_name: Option<&str>) -> syn::Stmt {
    match cleanup_name {
        Some(name) => syn::parse_quote! {
            __kobo_handler_guard.register_cleanup(#name);
        },
        None => syn::parse_quote! {
            __kobo_handler_guard.register_cleanup("");
        },
    }
}

fn cleanup_call_stmt(cleanup_hook: Option<&syn::Path>) -> syn::Stmt {
    match cleanup_hook {
        Some(path) => syn::parse_quote! {
            #path().await;
        },
        None => syn::parse_quote! {
            __kobo_handler_guard.note_no_cleanup_hook();
        },
    }
}

fn handler_source_doc(handler_name: &str, source_line: usize) -> String {
    format!("kobo: handler {handler_name} source_line={source_line}")
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
        struct KoboHandlerLifecycleGuard {
            handler: &'static str,
            source_line: usize,
            cleanup_hook: Option<&'static str>,
            state: &'static str,
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
                    state: "entered",
                }
            }

            fn register_cleanup(&mut self, cleanup_hook: &'static str) {
                self.cleanup_hook = if cleanup_hook.is_empty() {
                    None
                } else {
                    Some(cleanup_hook)
                };
            }

            fn note_no_cleanup_hook(&mut self) {
                self.cleanup_hook = self.cleanup_hook;
            }

            fn record_reply(&mut self) {
                self.state = "reply";
            }

            fn record_reject(&mut self) {
                self.state = "reject";
            }

            fn record_cancel(&mut self) {
                self.state = "cancel";
            }

            fn metrics_boundary(&self, _handler: &'static str) {}

            fn close_tracing_span(&self) {
                let _ = (self.handler, self.source_line, self.state);
            }

            fn run_cancel_cleanup_on_drop(&mut self) {
                if self.state == "entered" {
                    self.record_cancel();
                }
            }
        }
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
