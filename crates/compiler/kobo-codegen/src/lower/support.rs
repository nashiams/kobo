use syn::parse_quote;

use super::rewrite::ConcurrentSupportNeeds;

pub(crate) fn support_items(
    needs_rc: bool,
    needs_refcell: bool,
    needs_arc: bool,
    needs_scoped_handle: bool,
    needs_diag_owner: bool,
) -> Vec<syn::Item> {
    let mut items = Vec::new();

    if needs_diag_owner {
        // DiagOwner must come before Rc/RefCell imports so the reader sees the
        // wrapper type first. The `diag` feature is activated by the compiler
        // when KOBO_DIAG=1 is set at compile time via build.rs.
        // Otherwise the user must add `kobo-diag` to their Cargo.toml.
        items.push(parse_quote!(
            use kobo_diag::DiagOwner;
        ));
    }

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

pub(crate) fn insert_concurrent_support_items(file: &mut syn::File, needs: ConcurrentSupportNeeds) {
    let mut items = Vec::new();
    if needs.live_cell {
        items.extend(live_cell_items());
    }
    if needs.view_distance {
        items.extend(view_distance_items());
    }
    if items.is_empty() {
        return;
    }

    items.append(&mut file.items);
    file.items = items;
}

fn live_cell_items() -> Vec<syn::Item> {
    vec![
        parse_quote! {
            struct KoboArcSwap<T> {
                inner: arc_swap::ArcSwap<T>,
            }
        },
        parse_quote! {
            impl<T> KoboArcSwap<T> {
                fn from_pointee(value: T) -> Self {
                    Self {
                        inner: arc_swap::ArcSwap::from_pointee(value),
                    }
                }

                fn load(&self) -> std::sync::Arc<T> {
                    self.inner.load_full()
                }

                fn store(&self, value: std::sync::Arc<T>) {
                    self.inner.store(value);
                }
            }
        },
    ]
}

fn view_distance_items() -> Vec<syn::Item> {
    vec![
        parse_quote! {
            #[derive(Clone, Copy, Debug, Eq, PartialEq)]
            struct KoboViewDistance {
                cells: u32,
            }
        },
        parse_quote! {
            impl KoboViewDistance {
                fn new(cells: u32) -> Self {
                    Self { cells }
                }

                fn get(&self) -> u32 {
                    self.cells
                }

                fn contains_delta(&self, dx: i32, dy: i32) -> bool {
                    let distance_squared = i64::from(dx) * i64::from(dx)
                        + i64::from(dy) * i64::from(dy);
                    let radius = i64::from(self.cells);
                    distance_squared <= radius * radius
                }
            }
        },
    ]
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
