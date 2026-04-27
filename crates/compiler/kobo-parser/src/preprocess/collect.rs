/// Collect `@strict` blocks and `@strict fn` items from a parsed `syn::File`.
///
/// Must be called BEFORE `postprocess_strict_markers` strips the marker
/// attributes.
use syn::visit::Visit;

use crate::ast::{KoboBlock, KoboItemFn};
use kobo_ir::{FileId, KoboSpan};

use super::is_kobo_strict_attr;

/// Walk a `syn::File` (BEFORE stripping markers) and collect every
/// `@strict` block and `@strict fn` item as `KoboBlock` / `KoboItemFn`.
///
/// Must be called on the syn file BEFORE `postprocess_strict_markers`;
/// after stripping the `#[__kobo_strict]` attributes are gone and this
/// function would return empty vectors.
pub fn collect_strict_items_from_syn(
    file: &syn::File,
    source: &str,
    file_id: FileId,
) -> (Vec<KoboBlock>, Vec<KoboItemFn>) {
    let mut collector = StrictItemCollector::new(source, file_id);
    collector.visit_file(file);
    collector.finish()
}

struct StrictItemCollector<'src> {
    file_id: FileId,
    strict_blocks: Vec<KoboBlock>,
    strict_fns: Vec<KoboItemFn>,
    line_starts: Vec<usize>,
    source_len: usize,
    /// Depth of async context nesting (async fn or async {} block).
    async_depth: u32,
    /// Span of the outermost enclosing async context.
    async_context_span: Option<KoboSpan>,
    _source: std::marker::PhantomData<&'src str>,
}

impl<'src> StrictItemCollector<'src> {
    fn new(source: &'src str, file_id: FileId) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            file_id,
            strict_blocks: Vec::new(),
            strict_fns: Vec::new(),
            line_starts,
            source_len: source.len(),
            async_depth: 0,
            async_context_span: None,
            _source: std::marker::PhantomData,
        }
    }

    fn byte_offset(&self, lc: proc_macro2::LineColumn) -> u32 {
        let line_idx = lc.line.saturating_sub(1);
        let line_start = self
            .line_starts
            .get(line_idx)
            .copied()
            .unwrap_or(self.source_len);
        ((line_start + lc.column).min(self.source_len)) as u32
    }

    fn kobo_span_from(&self, span: proc_macro2::Span) -> KoboSpan {
        KoboSpan::new(
            self.byte_offset(span.start()),
            self.byte_offset(span.end()),
            self.file_id,
        )
    }

    fn finish(self) -> (Vec<KoboBlock>, Vec<KoboItemFn>) {
        (self.strict_blocks, self.strict_fns)
    }
}

impl<'ast, 'src> Visit<'ast> for StrictItemCollector<'src> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        use syn::spanned::Spanned;
        if node.attrs.iter().any(is_kobo_strict_attr) {
            self.strict_fns.push(KoboItemFn {
                inner: node.clone(),
                span: self.kobo_span_from(node.span()),
                is_strict: true,
                strict_keyword_span: None,
                is_async: node.sig.asyncness.is_some(),
            });
        }

        let is_async = node.sig.asyncness.is_some();
        if is_async {
            let prev_span = self.async_context_span;
            if self.async_depth == 0 {
                self.async_context_span = Some(self.kobo_span_from(node.span()));
            }
            self.async_depth += 1;
            syn::visit::visit_item_fn(self, node);
            self.async_depth -= 1;
            if self.async_depth == 0 {
                self.async_context_span = prev_span;
            }
        } else {
            syn::visit::visit_item_fn(self, node);
        }
    }

    fn visit_expr_async(&mut self, node: &'ast syn::ExprAsync) {
        use syn::spanned::Spanned;
        let prev_span = self.async_context_span;
        if self.async_depth == 0 {
            self.async_context_span = Some(self.kobo_span_from(node.span()));
        }
        self.async_depth += 1;
        syn::visit::visit_expr_async(self, node);
        self.async_depth -= 1;
        if self.async_depth == 0 {
            self.async_context_span = prev_span;
        }
    }

    fn visit_expr_block(&mut self, node: &'ast syn::ExprBlock) {
        use syn::spanned::Spanned;
        if node.attrs.iter().any(is_kobo_strict_attr) {
            self.strict_blocks.push(KoboBlock {
                body: node.block.clone(),
                span: self.kobo_span_from(node.block.span()),
                is_strict: true,
                strict_keyword_span: None,
                is_inside_async: self.async_depth > 0,
                async_context_span: self.async_context_span,
            });
        }
        syn::visit::visit_expr_block(self, node);
    }
}
