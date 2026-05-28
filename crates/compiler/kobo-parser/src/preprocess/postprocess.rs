/// Post-process: strip `#[__kobo_strict]` marker attributes from the AST.
///
/// Invariant C06: the marker never survives into the final AST.
use syn::visit_mut::VisitMut;

use super::{is_kobo_strict_attr, KeywordMarker, PreprocessError};

/// Walk the AST after `syn::parse_file()`, remove `#[__kobo_strict]` attributes,
/// and record which nodes were marked. Invariant C06: the marker never survives
/// into the final AST.
pub fn postprocess_strict_markers(
    file: &mut syn::File,
    _markers: &[KeywordMarker],
) -> Result<(), PreprocessError> {
    let mut remover = StrictMarkerRemover;
    remover.visit_file_mut(file);
    Ok(())
}

struct StrictMarkerRemover;

impl VisitMut for StrictMarkerRemover {
    fn visit_item_fn_mut(&mut self, node: &mut syn::ItemFn) {
        node.attrs.retain(|attr| !is_kobo_strict_attr(attr));
        syn::visit_mut::visit_item_fn_mut(self, node);
    }

    fn visit_expr_block_mut(&mut self, node: &mut syn::ExprBlock) {
        node.attrs.retain(|attr| !is_kobo_strict_attr(attr));
        syn::visit_mut::visit_expr_block_mut(self, node);
    }
}
