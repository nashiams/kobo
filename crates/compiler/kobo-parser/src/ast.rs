use std::collections::HashMap;

use kobo_ir::{FileId, KoboAstNodeId, KoboSpan};
use proc_macro2::{LineColumn, Span};

/// A parsed Kobo source file: the `syn::File` AST with stable node identity.
pub struct KoboFile {
    pub file_id: FileId,
    pub inner: syn::File,
    pub items: Vec<KoboAstNode>,
    source_len: usize,
    line_starts: Vec<usize>,
    bindings: Vec<KoboBinding>,
    binding_index_by_id: HashMap<KoboAstNodeId, usize>,
    binding_index_by_span: HashMap<KoboSpan, usize>,
    /// @strict blocks collected before marker-stripping (P3 / v0.5).
    strict_blocks: Vec<KoboBlock>,
    /// @strict fn items collected before marker-stripping (P3 / v0.5).
    strict_fns: Vec<KoboItemFn>,
}

/// A Kobo block that may carry `@strict` annotation.
///
/// Used by kobo-transform's strict analysis. Contract C09: `is_strict` is
/// set during parsing (P1); the field is the sole source of truth.
#[derive(Clone)]
pub struct KoboBlock {
    pub body: syn::Block,
    pub span: KoboSpan,
    pub is_strict: bool,
    /// Original byte offset of the `@strict` keyword, for diagnostic spans.
    pub strict_keyword_span: Option<KoboSpan>,
    /// True if this block is inside an async fn or async {} block (K0063).
    pub is_inside_async: bool,
    /// Span of the enclosing async fn or async block, for K0063 diagnostics.
    pub async_context_span: Option<KoboSpan>,
}

/// A Kobo function item that may carry `@strict` annotation.
///
/// Used by kobo-transform's strict analysis. `is_async` is true when the
/// function is declared with `async fn` (needed for K0063 detection).
#[derive(Clone)]
pub struct KoboItemFn {
    pub inner: syn::ItemFn,
    pub span: KoboSpan,
    pub is_strict: bool,
    pub strict_keyword_span: Option<KoboSpan>,
    pub is_async: bool,
}

/// A top-level item in the Kobo AST, wrapped with a stable ID and source span.
pub struct KoboAstNode {
    pub id: KoboAstNodeId,
    pub span: KoboSpan,
    pub inner: syn::Item,
}

/// Kind of ownership-relevant binding discovered during parse.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum KoboBindingKind {
    Parameter,
    Local,
    Const,
    Static,
    SelfParam,
    MutSelfParam,
}

/// Stable metadata for a concrete binding declaration.
#[derive(Clone)]
pub struct KoboBinding {
    pub id: KoboAstNodeId,
    pub span: KoboSpan,
    pub ident: syn::Ident,
    pub ty: Option<syn::Type>,
    pub kind: KoboBindingKind,
}

impl KoboFile {
    pub(crate) fn new(
        file_id: FileId,
        inner: syn::File,
        items: Vec<KoboAstNode>,
        source: &str,
        bindings: Vec<KoboBinding>,
    ) -> Self {
        let binding_index_by_id = bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| (binding.id, index))
            .collect();

        let binding_index_by_span = bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| (binding.span, index))
            .collect();

        let line_starts = build_line_starts(source);

        Self {
            file_id,
            inner,
            items,
            source_len: source.len(),
            line_starts,
            bindings,
            binding_index_by_id,
            binding_index_by_span,
            strict_blocks: Vec::new(),
            strict_fns: Vec::new(),
        }
    }

    /// Store @strict blocks/fns collected during preprocessing (before marker-stripping).
    pub fn set_strict_items(&mut self, blocks: Vec<KoboBlock>, fns: Vec<KoboItemFn>) {
        self.strict_blocks = blocks;
        self.strict_fns = fns;
    }

    /// Access the inner `syn::File`.
    pub fn syn_file(&self) -> &syn::File {
        &self.inner
    }

    /// Mutable access to the inner `syn::File` (for postprocess marker stripping).
    pub fn syn_file_mut(&mut self) -> &mut syn::File {
        &mut self.inner
    }

    /// All @strict blocks in this file (populated by `collect_strict_items_from_syn`).
    pub fn strict_blocks(&self) -> &[KoboBlock] {
        &self.strict_blocks
    }

    /// All @strict fn items in this file (populated by `collect_strict_items_from_syn`).
    pub fn strict_fns(&self) -> &[KoboItemFn] {
        &self.strict_fns
    }

    pub fn iter_bindings(&self) -> impl Iterator<Item = &KoboBinding> {
        self.bindings.iter()
    }

    /// S-3: Find AST binding IDs whose declared type matches any of the given engine struct names.
    pub fn engine_typed_binding_ids(&self, engine_names: &[String]) -> Vec<KoboAstNodeId> {
        self.bindings
            .iter()
            .filter_map(|b| {
                let ty = b.ty.as_ref()?;
                let ty_name = type_path_name(ty)?;
                if engine_names
                    .iter()
                    .any(|name| ty_name.contains(name.as_str()))
                {
                    Some(b.id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn binding_for_id(&self, id: KoboAstNodeId) -> Option<&KoboBinding> {
        let index = *self.binding_index_by_id.get(&id)?;
        self.bindings.get(index)
    }

    pub fn binding_for_span(&self, span: KoboSpan) -> Option<&KoboBinding> {
        let index = *self.binding_index_by_span.get(&span)?;
        self.bindings.get(index)
    }

    pub fn span_from_syn(&self, span: Span) -> KoboSpan {
        KoboSpan::new(
            self.byte_offset(span.start()) as u32,
            self.byte_offset(span.end()) as u32,
            self.file_id,
        )
    }

    /// Line start byte offsets (for external span conversion).
    pub fn source_line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// Total source length in bytes.
    pub fn source_len(&self) -> usize {
        self.source_len
    }

    pub fn line_col(&self, span: KoboSpan) -> (usize, usize) {
        let offset = span.start as usize;
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index];

        (line_index + 1, offset.saturating_sub(line_start) + 1)
    }

    fn byte_offset(&self, line_column: LineColumn) -> usize {
        let line_index = line_column.line.saturating_sub(1);
        let line_start = self
            .line_starts
            .get(line_index)
            .copied()
            .unwrap_or(self.source_len);

        (line_start + line_column.column).min(self.source_len)
    }
}

fn build_line_starts(source: &str) -> Vec<usize> {
    let mut line_starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(index + 1);
        }
    }

    line_starts
}

/// Extract the path string from a syn::Type for engine struct matching.
fn type_path_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => {
            let name = tp
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            Some(name)
        }
        _ => None,
    }
}
