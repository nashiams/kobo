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
        }
    }

    pub fn iter_bindings(&self) -> impl Iterator<Item = &KoboBinding> {
        self.bindings.iter()
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
