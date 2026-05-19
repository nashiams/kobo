use quote::{format_ident, quote};
use syn::spanned::Spanned;

use crate::{RuntimeProfileOptions, ServiceRuntimeEvidence, ServiceRuntimeMethodEvidence};

#[derive(Clone)]
struct ServiceSpec {
    service_ident: syn::Ident,
    message_ident: syn::Ident,
    handle_ident: syn::Ident,
    buffer_size: usize,
    source_line: usize,
    runtime_profile: RuntimeProfileOptions,
    methods: Vec<ServiceMethod>,
}

#[derive(Clone)]
struct ServiceMethod {
    method_ident: syn::Ident,
    variant_ident: syn::Ident,
    fields: Vec<ServiceField>,
    reply_ty: syn::Type,
}

#[derive(Clone)]
struct ServiceField {
    ident: syn::Ident,
    ty: syn::Type,
}

pub(crate) struct ServiceSupport {
    pub(crate) items: Vec<syn::Item>,
    pub(crate) evidence: Vec<ServiceRuntimeEvidence>,
}

pub(crate) fn service_support(
    ast: &kobo_parser::KoboFile,
    file: &syn::File,
    runtime_profile: &RuntimeProfileOptions,
) -> ServiceSupport {
    let specs = service_specs(ast, file, runtime_profile);
    if specs.is_empty() {
        return ServiceSupport {
            items: Vec::new(),
            evidence: Vec::new(),
        };
    }

    let mut items = Vec::new();
    items.extend(shared_service_runtime_items());
    let mut evidence = Vec::new();
    for spec in specs {
        evidence.push(service_runtime_evidence(&spec));
        items.extend(items_for_service(&spec));
    }
    ServiceSupport { items, evidence }
}

pub(crate) fn append_service_support_items(file: &mut syn::File, items: Vec<syn::Item>) {
    file.items.extend(items);
}

fn service_specs(
    ast: &kobo_parser::KoboFile,
    file: &syn::File,
    runtime_profile: &RuntimeProfileOptions,
) -> Vec<ServiceSpec> {
    file.items
        .iter()
        .filter_map(|item| {
            let syn::Item::Impl(item_impl) = item else {
                return None;
            };
            service_spec_from_impl(ast, item_impl, runtime_profile)
        })
        .collect()
}

fn service_spec_from_impl(
    ast: &kobo_parser::KoboFile,
    item_impl: &syn::ItemImpl,
    runtime_profile: &RuntimeProfileOptions,
) -> Option<ServiceSpec> {
    let attr = service_attr(&item_impl.attrs)?;
    let buffer_size = service_buffer_size(attr).unwrap_or(runtime_profile.service_buffer);
    let service_ident = service_ident_from_self_ty(&item_impl.self_ty)?;
    let service_name = service_ident.to_string();
    let methods = service_methods(item_impl);
    if methods.is_empty() {
        return None;
    }

    Some(ServiceSpec {
        service_ident,
        message_ident: format_ident!("{}Message", service_name),
        handle_ident: format_ident!("{}Service", service_name),
        buffer_size,
        source_line: service_source_line(ast, attr),
        runtime_profile: runtime_profile.clone(),
        methods,
    })
}

fn service_attr(attrs: &[syn::Attribute]) -> Option<&syn::Attribute> {
    attrs.iter().find(|attr| is_service_attr(attr))
}

fn service_buffer_size(attr: &syn::Attribute) -> Option<usize> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let compact = list.tokens.to_string().replace(' ', "");
    compact
        .strip_prefix("buffer=")
        .and_then(|rest| rest.parse::<usize>().ok())
}

fn service_source_line(ast: &kobo_parser::KoboFile, attr: &syn::Attribute) -> usize {
    let span = ast.span_from_syn(attr.span());
    let (line, _) = ast.line_col(span);
    line
}

fn is_service_attr(attr: &syn::Attribute) -> bool {
    let segments = attr.path().segments.iter().collect::<Vec<_>>();
    segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "service"
}

fn service_ident_from_self_ty(self_ty: &syn::Type) -> Option<syn::Ident> {
    let syn::Type::Path(type_path) = self_ty else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.clone())
}

fn service_methods(item_impl: &syn::ItemImpl) -> Vec<ServiceMethod> {
    item_impl
        .items
        .iter()
        .filter_map(|item| {
            let syn::ImplItem::Fn(method) = item else {
                return None;
            };
            service_method(method)
        })
        .collect()
}

fn service_method(method: &syn::ImplItemFn) -> Option<ServiceMethod> {
    if method.sig.asyncness.is_none() {
        return None;
    }

    Some(ServiceMethod {
        method_ident: method.sig.ident.clone(),
        variant_ident: format_ident!("{}", upper_camel_case(&method.sig.ident.to_string())),
        fields: service_fields(method),
        reply_ty: reply_type(&method.sig.output),
    })
}

fn service_fields(method: &syn::ImplItemFn) -> Vec<ServiceField> {
    method
        .sig
        .inputs
        .iter()
        .filter_map(|input| {
            let syn::FnArg::Typed(argument) = input else {
                return None;
            };
            let syn::Pat::Ident(pat_ident) = argument.pat.as_ref() else {
                return None;
            };
            Some(ServiceField {
                ident: pat_ident.ident.clone(),
                ty: (*argument.ty).clone(),
            })
        })
        .collect()
}

fn reply_type(output: &syn::ReturnType) -> syn::Type {
    match output {
        syn::ReturnType::Default => syn::parse_quote!(()),
        syn::ReturnType::Type(_, ty) => ty.as_ref().clone(),
    }
}

fn upper_camel_case(value: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for ch in value.chars() {
        if ch == '_' {
            capitalize_next = true;
            continue;
        }
        if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

fn shared_service_runtime_items() -> Vec<syn::Item> {
    let mut items = Vec::new();
    if let Some(item) = syn::parse2(quote! {
        #[derive(Clone, Default)]
        pub struct KoboServiceCancellationToken {
            cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }
    })
    .ok()
    {
        items.push(item);
    }
    if let Some(item) = syn::parse2(quote! {
        impl KoboServiceCancellationToken {
            pub fn new() -> Self {
                Self::default()
            }

            pub fn cancel(&self) {
                self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            }

            pub fn is_cancelled(&self) -> bool {
                self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
            }
        }
    })
    .ok()
    {
        items.push(item);
    }
    if let Some(item) = syn::parse2(quote! {
        #[derive(Clone, Debug)]
        struct KoboServiceScenarioHookEvent {
            phase: &'static str,
            service: &'static str,
            method: &'static str,
        }
    })
    .ok()
    {
        items.push(item);
    }
    if let Some(item) = syn::parse2(quote! {
        static KOBO_SERVICE_SCENARIO_HOOKS: std::sync::OnceLock<
            std::sync::Mutex<Vec<KoboServiceScenarioHookEvent>>,
        > = std::sync::OnceLock::new();
    })
    .ok()
    {
        items.push(item);
    }
    if let Some(item) = syn::parse2(quote! {
        struct KoboServiceScenarioHook;
    })
    .ok()
    {
        items.push(item);
    }
    if let Some(item) = syn::parse2(quote! {
        impl KoboServiceScenarioHook {
            fn store() -> &'static std::sync::Mutex<Vec<KoboServiceScenarioHookEvent>> {
                KOBO_SERVICE_SCENARIO_HOOKS
                    .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            }

            fn record(phase: &'static str, service: &'static str, method: &'static str) {
                if let Ok(mut events) = Self::store().lock() {
                    events.push(KoboServiceScenarioHookEvent {
                        phase,
                        service,
                        method,
                    });
                }
                println!(
                    "KOBO_SERVICE_HOOK:{{\"phase\":\"{}\",\"service\":\"{}\",\"method\":\"{}\"}}",
                    phase,
                    service,
                    method
                );
                println!(
                    "KOBO_EVENT:{{\"kind\":\"service-scheduler-hook\",\"label\":\"{}::{}:{}\",\"value\":null}}",
                    service,
                    method,
                    phase
                );
            }

            fn before(service: &'static str, method: &'static str) {
                Self::record("before", service, method);
            }

            fn after(service: &'static str, method: &'static str) {
                Self::record("after", service, method);
            }

            fn shutdown(service: &'static str) {
                Self::record("shutdown", service, "shutdown");
            }

            fn events() -> Vec<KoboServiceScenarioHookEvent> {
                Self::store()
                    .lock()
                    .map(|events| events.clone())
                    .unwrap_or_default()
            }

            fn clear() {
                if let Ok(mut events) = Self::store().lock() {
                    events.clear();
                }
            }
        }
    })
    .ok()
    {
        items.push(item);
    }
    items
}

fn items_for_service(spec: &ServiceSpec) -> Vec<syn::Item> {
    let mut items = Vec::new();
    if let Some(item) = message_enum_item(spec) {
        items.push(item);
    }
    if let Some(item) = service_handle_struct_item(spec) {
        items.push(item);
    }
    if let Some(item) = service_error_item(spec) {
        items.push(item);
    }
    if let Some(item) = service_handle_impl_item(spec) {
        items.push(item);
    }
    items
}

fn message_enum_item(spec: &ServiceSpec) -> Option<syn::Item> {
    let message_ident = &spec.message_ident;
    let service_name = spec.service_ident.to_string();
    let source_line = spec.source_line;
    let source_doc = format!("kobo: service {service_name} source_line={source_line}");
    let runtime_doc = spec.runtime_profile.inspect_comment();
    let variants = spec.methods.iter().map(message_variant_tokens);
    syn::parse2(quote! {
        #[doc = #source_doc]
        #[doc = #runtime_doc]
        pub enum #message_ident {
            #(#variants,)*
            Shutdown,
        }
    })
    .ok()
}

fn message_variant_tokens(method: &ServiceMethod) -> proc_macro2::TokenStream {
    let variant_ident = &method.variant_ident;
    let fields = method.fields.iter().map(field_tokens);
    let reply_ty = &method.reply_ty;
    quote! {
        #variant_ident {
            #(#fields,)*
            __reply: tokio::sync::oneshot::Sender<#reply_ty>
        }
    }
}

fn field_tokens(field: &ServiceField) -> proc_macro2::TokenStream {
    let ident = &field.ident;
    let ty = &field.ty;
    quote!(#ident: #ty)
}

fn service_handle_struct_item(spec: &ServiceSpec) -> Option<syn::Item> {
    let message_ident = &spec.message_ident;
    let handle_ident = &spec.handle_ident;
    let service_name = spec.service_ident.to_string();
    let source_line = spec.source_line;
    let source_doc = format!("kobo: service {service_name} source_line={source_line}");
    let runtime_doc = spec.runtime_profile.inspect_comment();
    syn::parse2(quote! {
        #[doc = #source_doc]
        #[doc = #runtime_doc]
        pub struct #handle_ident {
            sender: tokio::sync::mpsc::Sender<#message_ident>,
            cancellation: KoboServiceCancellationToken,
        }
    })
    .ok()
}

fn service_handle_impl_item(spec: &ServiceSpec) -> Option<syn::Item> {
    let message_ident = &spec.message_ident;
    let handle_ident = &spec.handle_ident;
    let buffer_size = syn::LitInt::new(
        &spec.buffer_size.to_string(),
        proc_macro2::Span::call_site(),
    );
    let service_ident = &spec.service_ident;
    let error_ident = service_error_ident(spec);
    let dispatch_arms = spec
        .methods
        .iter()
        .map(|method| dispatch_arm_tokens(spec, method));
    let client_methods = spec
        .methods
        .iter()
        .map(|method| client_method_tokens(spec, method));
    let send_method = if spec.runtime_profile.service_backpressure == "try-send" {
        quote! {
            async fn send(
                &self,
                message: #message_ident,
            ) -> Result<(), tokio::sync::mpsc::error::TrySendError<#message_ident>> {
                self.sender.try_send(message)
            }
        }
    } else {
        quote! {
            async fn send(
                &self,
                message: #message_ident,
            ) -> Result<(), tokio::sync::mpsc::error::SendError<#message_ident>> {
                self.sender.send(message).await
            }
        }
    };
    syn::parse2(quote! {
        impl #handle_ident {
            pub fn new(sender: tokio::sync::mpsc::Sender<#message_ident>) -> Self {
                Self::new_with_cancellation(sender, KoboServiceCancellationToken::new())
            }

            pub fn new_with_cancellation(
                sender: tokio::sync::mpsc::Sender<#message_ident>,
                cancellation: KoboServiceCancellationToken,
            ) -> Self {
                Self {
                    sender,
                    cancellation,
                }
            }

            pub fn channel() -> (
                tokio::sync::mpsc::Sender<#message_ident>,
                tokio::sync::mpsc::Receiver<#message_ident>,
            ) {
                tokio::sync::mpsc::channel::<#message_ident>(#buffer_size)
            }

            pub fn start(service: #service_ident) -> (Self, tokio::task::JoinHandle<()>) {
                let (sender, receiver) = Self::channel();
                let cancellation = KoboServiceCancellationToken::new();
                let handle = Self::new_with_cancellation(sender, cancellation.clone());
                let worker = tokio::spawn(Self::serve(service, receiver, cancellation));
                (handle, worker)
            }

            async fn serve(
                mut service: #service_ident,
                mut receiver: tokio::sync::mpsc::Receiver<#message_ident>,
                cancellation: KoboServiceCancellationToken,
            ) {
                while let Some(message) = receiver.recv().await {
                    match message {
                        #(#dispatch_arms)*
                        #message_ident::Shutdown => {
                            cancellation.cancel();
                            KoboServiceScenarioHook::shutdown(stringify!(#service_ident));
                            break;
                        }
                    }
                }
            }

            #send_method

            #(#client_methods)*

            pub async fn shutdown(
                &self,
            ) -> Result<(), tokio::sync::mpsc::error::SendError<#message_ident>> {
                self.sender.send(#message_ident::Shutdown).await
            }

            pub async fn shutdown_and_wait(
                self,
                worker: tokio::task::JoinHandle<()>,
            ) -> Result<(), #error_ident> {
                self.shutdown().await.map_err(|_| #error_ident::SendClosed)?;
                worker.await.map_err(|_| #error_ident::JoinFailed)
            }

            pub fn is_shutdown_requested(&self) -> bool {
                self.cancellation.is_cancelled()
            }

            pub fn service_type_name(&self) -> &'static str {
                stringify!(#service_ident)
            }
        }
    })
    .ok()
}

fn service_error_item(spec: &ServiceSpec) -> Option<syn::Item> {
    let error_ident = service_error_ident(spec);
    syn::parse2(quote! {
        #[derive(Debug)]
        pub enum #error_ident {
            SendClosed,
            ResponseDropped,
            JoinFailed,
        }
    })
    .ok()
}

fn service_error_ident(spec: &ServiceSpec) -> syn::Ident {
    format_ident!("{}ServiceError", spec.service_ident)
}

fn dispatch_arm_tokens(spec: &ServiceSpec, method: &ServiceMethod) -> proc_macro2::TokenStream {
    let message_ident = &spec.message_ident;
    let service_ident = &spec.service_ident;
    let variant_ident = &method.variant_ident;
    let method_ident = &method.method_ident;
    let method_name = method_ident.to_string();
    let field_idents = method
        .fields
        .iter()
        .map(|field| &field.ident)
        .collect::<Vec<_>>();
    quote! {
        #message_ident::#variant_ident { #(#field_idents,)* __reply } => {
            if cancellation.is_cancelled() {
                break;
            }
            KoboServiceScenarioHook::before(stringify!(#service_ident), #method_name);
            let __kobo_reply = service.#method_ident(#(#field_idents),*).await;
            let _ = __reply.send(__kobo_reply);
            KoboServiceScenarioHook::after(stringify!(#service_ident), #method_name);
        }
    }
}

fn client_method_tokens(spec: &ServiceSpec, method: &ServiceMethod) -> proc_macro2::TokenStream {
    let message_ident = &spec.message_ident;
    let error_ident = service_error_ident(spec);
    let method_ident = &method.method_ident;
    let variant_ident = &method.variant_ident;
    let args = method.fields.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        quote!(#ident: #ty)
    });
    let field_idents = method
        .fields
        .iter()
        .map(|field| &field.ident)
        .collect::<Vec<_>>();
    let reply_ty = &method.reply_ty;
    quote! {
        pub async fn #method_ident(&self #(, #args)*) -> Result<#reply_ty, #error_ident> {
            let (__reply_tx, __reply_rx) = tokio::sync::oneshot::channel();
            self.send(#message_ident::#variant_ident {
                #(#field_idents,)*
                __reply: __reply_tx,
            })
            .await
            .map_err(|_| #error_ident::SendClosed)?;
            __reply_rx.await.map_err(|_| #error_ident::ResponseDropped)
        }
    }
}

fn service_runtime_evidence(spec: &ServiceSpec) -> ServiceRuntimeEvidence {
    ServiceRuntimeEvidence {
        name: spec.service_ident.to_string(),
        buffer: spec.buffer_size,
        source_line: spec.source_line,
        backpressure: spec.runtime_profile.service_backpressure.clone(),
        dispatch_loop: true,
        client_api: true,
        scenario_hooks: true,
        hook_events: ["before", "after", "shutdown"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        methods: spec
            .methods
            .iter()
            .map(|method| ServiceRuntimeMethodEvidence {
                name: method.method_ident.to_string(),
                variant: method.variant_ident.to_string(),
            })
            .collect(),
    }
}
