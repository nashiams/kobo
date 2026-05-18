use quote::{format_ident, quote};

#[derive(Clone)]
struct ServiceSpec {
    service_ident: syn::Ident,
    message_ident: syn::Ident,
    handle_ident: syn::Ident,
    buffer_size: usize,
    methods: Vec<ServiceMethod>,
}

#[derive(Clone)]
struct ServiceMethod {
    variant_ident: syn::Ident,
    fields: Vec<ServiceField>,
    reply_ty: syn::Type,
}

#[derive(Clone)]
struct ServiceField {
    ident: syn::Ident,
    ty: syn::Type,
}

pub(crate) fn service_support_items(file: &syn::File) -> Vec<syn::Item> {
    let specs = service_specs(file);
    if specs.is_empty() {
        return Vec::new();
    }

    let mut items = Vec::new();
    items.extend(cancellation_token_items());
    for spec in specs {
        items.extend(items_for_service(&spec));
    }
    items
}

pub(crate) fn append_service_support_items(file: &mut syn::File, items: Vec<syn::Item>) {
    file.items.extend(items);
}

fn service_specs(file: &syn::File) -> Vec<ServiceSpec> {
    file.items
        .iter()
        .filter_map(|item| {
            let syn::Item::Impl(item_impl) = item else {
                return None;
            };
            service_spec_from_impl(item_impl)
        })
        .collect()
}

fn service_spec_from_impl(item_impl: &syn::ItemImpl) -> Option<ServiceSpec> {
    let buffer_size = service_buffer_size(&item_impl.attrs)?;
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
        methods,
    })
}

fn service_buffer_size(attrs: &[syn::Attribute]) -> Option<usize> {
    let attr = attrs.iter().find(|attr| is_service_attr(attr))?;
    let syn::Meta::List(list) = &attr.meta else {
        return Some(64);
    };
    let compact = list.tokens.to_string().replace(' ', "");
    let Some(rest) = compact.strip_prefix("buffer=") else {
        return Some(64);
    };
    rest.parse::<usize>().ok()
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

fn cancellation_token_items() -> Vec<syn::Item> {
    let mut items = Vec::new();
    if let Some(item) = syn::parse2(quote! {
        #[derive(Clone, Default)]
        struct KoboServiceCancellationToken {
            cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }
    })
    .ok()
    {
        items.push(item);
    }
    if let Some(item) = syn::parse2(quote! {
        impl KoboServiceCancellationToken {
            fn new() -> Self {
                Self::default()
            }

            fn cancel(&self) {
                self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            }

            fn is_cancelled(&self) -> bool {
                self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
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
    if let Some(item) = service_handle_impl_item(spec) {
        items.push(item);
    }
    items
}

fn message_enum_item(spec: &ServiceSpec) -> Option<syn::Item> {
    let message_ident = &spec.message_ident;
    let variants = spec.methods.iter().map(message_variant_tokens);
    syn::parse2(quote! {
        enum #message_ident {
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
    syn::parse2(quote! {
        struct #handle_ident {
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
    syn::parse2(quote! {
        impl #handle_ident {
            fn new(sender: tokio::sync::mpsc::Sender<#message_ident>) -> Self {
                Self {
                    sender,
                    cancellation: KoboServiceCancellationToken::new(),
                }
            }

            fn channel() -> (
                tokio::sync::mpsc::Sender<#message_ident>,
                tokio::sync::mpsc::Receiver<#message_ident>,
            ) {
                tokio::sync::mpsc::channel::<#message_ident>(#buffer_size)
            }

            async fn send(
                &self,
                message: #message_ident,
            ) -> Result<(), tokio::sync::mpsc::error::SendError<#message_ident>> {
                self.sender.send(message).await
            }

            async fn shutdown(
                &self,
            ) -> Result<(), tokio::sync::mpsc::error::SendError<#message_ident>> {
                self.cancellation.cancel();
                self.sender.send(#message_ident::Shutdown).await
            }

            fn is_shutdown_requested(&self) -> bool {
                self.cancellation.is_cancelled()
            }

            fn service_type_name(&self) -> &'static str {
                stringify!(#service_ident)
            }
        }
    })
    .ok()
}
