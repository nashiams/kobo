use syn::parse_quote;

pub(crate) fn support_items(
    needs_rc: bool,
    needs_refcell: bool,
    needs_arc: bool,
    needs_scoped_handle: bool,
) -> Vec<syn::Item> {
    let mut items = Vec::new();

    if needs_refcell {
        items.push(parse_quote!(
            use std::cell::RefCell;
        ));
    }

    if needs_rc {
        items.push(parse_quote!(
            use std::rc::Rc;
        ));
    }

    if needs_arc {
        items.push(parse_quote!(
            use std::sync::Arc;
        ));
    }

    if needs_scoped_handle {
        items.extend(scoped_handle_items());
    }

    items
}

fn scoped_handle_items() -> Vec<syn::Item> {
    vec![
        parse_quote!(
            struct ScopedHandle<T>(T);
        ),
        parse_quote!(
            impl<T> ScopedHandle<T> {
                fn new(inner: T) -> Self {
                    Self(inner)
                }
            }
        ),
        parse_quote!(
            impl<T> std::ops::Deref for ScopedHandle<T> {
                type Target = T;

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }
        ),
        parse_quote!(
            impl<T> std::ops::DerefMut for ScopedHandle<T> {
                fn deref_mut(&mut self) -> &mut Self::Target {
                    &mut self.0
                }
            }
        ),
    ]
}
