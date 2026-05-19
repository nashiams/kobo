use quote::ToTokens;
use syn::visit::{self, Visit};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceSignatureWarning {
    pub service_name: String,
    pub method_name: Option<String>,
    pub reason: ServiceSignatureWarningReason,
    pub source_offset: usize,
    pub span_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceSignatureWarningReason {
    GenericService,
    GenericMethod,
    BorrowedMessageType,
    UnsupportedReceiver,
}

pub fn scan_source_service_signature_warnings(source: &str) -> Vec<ServiceSignatureWarning> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut scanner = ServiceSignatureScanner {
        source,
        warnings: Vec::new(),
    };
    scanner.visit_file(&file);
    scanner.warnings
}

struct ServiceSignatureScanner<'a> {
    source: &'a str,
    warnings: Vec<ServiceSignatureWarning>,
}

impl<'ast> Visit<'ast> for ServiceSignatureScanner<'_> {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if !item.attrs.iter().any(is_service_attr) {
            visit::visit_item_impl(self, item);
            return;
        }

        let service_name =
            service_name_from_self_ty(&item.self_ty).unwrap_or_else(|| "service".to_owned());
        if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
            self.push_warning(
                &service_name,
                None,
                ServiceSignatureWarningReason::GenericService,
                item.impl_token.span,
                service_name.len().max("impl".len()),
            );
        }
        if self_type_has_generic_arguments(&item.self_ty) {
            self.push_warning(
                &service_name,
                None,
                ServiceSignatureWarningReason::GenericService,
                item.self_ty.to_token_stream().to_string(),
                service_name.len(),
            );
        }

        for impl_item in &item.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if method.sig.asyncness.is_none() {
                continue;
            }
            self.inspect_method(&service_name, method);
        }
        visit::visit_item_impl(self, item);
    }
}

impl ServiceSignatureScanner<'_> {
    fn inspect_method(&mut self, service_name: &str, method: &syn::ImplItemFn) {
        let method_name = method.sig.ident.to_string();
        if !receiver_is_supported(&method.sig.inputs) {
            self.push_warning(
                service_name,
                Some(&method_name),
                ServiceSignatureWarningReason::UnsupportedReceiver,
                method.sig.ident.span(),
                method_name.len(),
            );
        }
        if !method.sig.generics.params.is_empty() || method.sig.generics.where_clause.is_some() {
            self.push_warning(
                service_name,
                Some(&method_name),
                ServiceSignatureWarningReason::GenericMethod,
                method.sig.ident.span(),
                method_name.len(),
            );
        }
        for input in &method.sig.inputs {
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            if type_crosses_borrowed_channel_boundary(argument.ty.as_ref()) {
                self.push_warning(
                    service_name,
                    Some(&method_name),
                    ServiceSignatureWarningReason::BorrowedMessageType,
                    argument.ty.to_token_stream().to_string(),
                    method_name.len(),
                );
            }
        }
        if let syn::ReturnType::Type(_, ty) = &method.sig.output {
            if type_crosses_borrowed_channel_boundary(ty.as_ref()) {
                self.push_warning(
                    service_name,
                    Some(&method_name),
                    ServiceSignatureWarningReason::BorrowedMessageType,
                    ty.to_token_stream().to_string(),
                    method_name.len(),
                );
            }
        }
    }

    fn push_warning(
        &mut self,
        service_name: &str,
        method_name: Option<&str>,
        reason: ServiceSignatureWarningReason,
        needle: impl OffsetNeedle,
        span_len: usize,
    ) {
        let source_offset = needle.offset(self.source).unwrap_or_else(|| {
            method_name
                .and_then(|name| ident_offset(self.source, name))
                .or_else(|| ident_offset(self.source, service_name))
                .unwrap_or(0)
        });
        let warning = ServiceSignatureWarning {
            service_name: service_name.to_owned(),
            method_name: method_name.map(str::to_owned),
            reason,
            source_offset,
            span_len: span_len.max(1),
        };
        if !self.warnings.iter().any(|seen| seen == &warning) {
            self.warnings.push(warning);
        }
    }
}

trait OffsetNeedle {
    fn offset(&self, source: &str) -> Option<usize>;
}

impl OffsetNeedle for proc_macro2::Span {
    fn offset(&self, _source: &str) -> Option<usize> {
        None
    }
}

impl OffsetNeedle for String {
    fn offset(&self, source: &str) -> Option<usize> {
        source.find(self)
    }
}

fn is_service_attr(attr: &syn::Attribute) -> bool {
    let segments = attr.path().segments.iter().collect::<Vec<_>>();
    segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "service"
}

fn service_name_from_self_ty(self_ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = self_ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn self_type_has_generic_arguments(self_ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = self_ty else {
        return false;
    };
    path.path.segments.iter().any(|segment| {
        !matches!(
            segment.arguments,
            syn::PathArguments::None | syn::PathArguments::Parenthesized(_)
        )
    })
}

fn receiver_is_supported(inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::Token![,]>) -> bool {
    let Some(first) = inputs.first() else {
        return false;
    };
    matches!(
        first,
        syn::FnArg::Receiver(receiver)
            if receiver.reference.is_some()
                && receiver.colon_token.is_none()
                && receiver.ty.to_token_stream().to_string().contains("Self")
    )
}

fn type_crosses_borrowed_channel_boundary(ty: &syn::Type) -> bool {
    if matches!(ty, syn::Type::Reference(_)) {
        return true;
    }
    let tokens = ty.to_token_stream().to_string();
    tokens.contains('&') || tokens.split_whitespace().any(|part| part.starts_with('\''))
}

fn ident_offset(source: &str, ident: &str) -> Option<usize> {
    source.match_indices(ident).find_map(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after = source[index + ident.len()..].chars().next();
        (!before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)).then_some(index)
    })
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_generic_service_and_borrowed_method_boundary() {
        let warnings = scan_source_service_signature_warnings(
            r#"
struct Cache<T> { value: T }

#[kobo::service]
impl<T> Cache<T> {
    async fn lookup<'a>(&self, key: &'a str) -> &'a str {
        key
    }
}
"#,
        );

        assert!(warnings.iter().any(|warning| {
            warning.reason == ServiceSignatureWarningReason::GenericService
                && warning.service_name == "Cache"
        }));
        assert!(warnings.iter().any(|warning| {
            warning.reason == ServiceSignatureWarningReason::GenericMethod
                && warning.method_name.as_deref() == Some("lookup")
        }));
        assert!(warnings.iter().any(|warning| {
            warning.reason == ServiceSignatureWarningReason::BorrowedMessageType
                && warning.method_name.as_deref() == Some("lookup")
        }));
    }
}
