/// Validate `#[kobo::handler]` attribute usage.
///
/// Rules:
/// - `#[kobo::handler]` is only valid on `async fn` items.
/// - Applying it to a non-async fn is an error.
/// - The attribute is stripped during codegen (handled by `strip_kobo_attrs`).

/// Check all items in a syn File for invalid `#[kobo::handler]` usage.
///
/// Returns `Ok(())` if all usages are valid, or `Err(message)` describing the violation.
pub fn validate_handler_attributes(file: &syn::File) -> Result<(), String> {
    for item in &file.items {
        validate_item_handler(item)?;
    }
    Ok(())
}

fn validate_item_handler(item: &syn::Item) -> Result<(), String> {
    match item {
        syn::Item::Fn(func) => {
            if has_kobo_handler_attr(&func.attrs) && func.sig.asyncness.is_none() {
                return Err(format!(
                    "#[kobo::handler] requires an async function, but `{}` is not async",
                    func.sig.ident
                ));
            }
        }
        syn::Item::Impl(item_impl) => {
            for impl_item in &item_impl.items {
                if let syn::ImplItem::Fn(method) = impl_item {
                    if has_kobo_handler_attr(&method.attrs) && method.sig.asyncness.is_none() {
                        return Err(format!(
                            "#[kobo::handler] requires an async function, but `{}` is not async",
                            method.sig.ident
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn has_kobo_handler_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        segments.len() == 2
            && segments[0].ident == "kobo"
            && segments[1].ident == "handler"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_validate(code: &str) -> Result<(), String> {
        let file = syn::parse_str::<syn::File>(code)
            .map_err(|e| format!("parse error: {e}"))?;
        validate_handler_attributes(&file)
    }

    #[test]
    fn async_fn_with_handler_is_valid() {
        let code = r#"
            #[kobo::handler]
            async fn handle(req: Request) -> Response {
                Response::ok("hello")
            }
        "#;
        assert!(parse_and_validate(code).is_ok());
    }

    #[test]
    fn sync_fn_with_handler_is_error() {
        let code = r#"
            #[kobo::handler]
            fn handle(req: Request) -> Response {
                Response::ok("hello")
            }
        "#;
        let result = parse_and_validate(code);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("handler"));
        assert!(err.contains("async"));
    }

    #[test]
    fn fn_without_handler_is_valid() {
        let code = r#"
            fn regular() -> i32 {
                42
            }
        "#;
        assert!(parse_and_validate(code).is_ok());
    }

    #[test]
    fn sync_method_with_handler_is_error() {
        let code = r#"
            impl Server {
                #[kobo::handler]
                fn handle(&self, req: Request) -> Response {
                    Response::ok("hello")
                }
            }
        "#;
        let result = parse_and_validate(code);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("handler"));
    }

    #[test]
    fn async_method_with_handler_is_valid() {
        let code = r#"
            impl Server {
                #[kobo::handler]
                async fn handle(&self, req: Request) -> Response {
                    Response::ok("hello")
                }
            }
        "#;
        assert!(parse_and_validate(code).is_ok());
    }
}
