use kobo_ir::{FileId, KoboSpan, NodeIdGen};
use proc_macro2::{LineColumn, Span};
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::ast::{KoboAstNode, KoboBinding, KoboBindingKind, KoboFile};

/// Errors produced during parsing.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("syn parse error: {message}")]
    Syn { span: KoboSpan, message: String },
}

impl ParseError {
    pub fn primary_span(&self) -> KoboSpan {
        match self {
            Self::Syn { span, .. } => *span,
        }
    }
}

/// Parses a `.kobo` source file using `syn::parse_file`.
///
/// Assigns stable IDs to every top-level item and every ownership-relevant
/// binding declaration discovered during parse.
pub fn parse_file(
    source: &str,
    file_id: FileId,
    id_gen: &mut NodeIdGen,
) -> Result<KoboFile, ParseError> {
    let line_index = LineIndex::new(source);
    let inner = syn::parse_file(source).map_err(|error| ParseError::Syn {
        span: line_index.span_from_syn(error.span(), file_id),
        message: error.to_string(),
    })?;

    let items = inner
        .items
        .iter()
        .map(|item| KoboAstNode {
            id: id_gen.next_ast_id(),
            span: line_index.span_from_syn(item.span(), file_id),
            inner: item.clone(),
        })
        .collect();

    let mut binding_collector = BindingCollector::new(file_id, &line_index, id_gen);
    binding_collector.visit_file(&inner);

    Ok(KoboFile::new(
        file_id,
        inner,
        items,
        source,
        binding_collector.finish(),
    ))
}

struct BindingCollector<'a> {
    file_id: FileId,
    line_index: &'a LineIndex,
    id_gen: &'a mut NodeIdGen,
    bindings: Vec<KoboBinding>,
}

impl<'a> BindingCollector<'a> {
    fn new(file_id: FileId, line_index: &'a LineIndex, id_gen: &'a mut NodeIdGen) -> Self {
        Self {
            file_id,
            line_index,
            id_gen,
            bindings: Vec::new(),
        }
    }

    fn finish(self) -> Vec<KoboBinding> {
        self.bindings
    }

    fn push_binding(&mut self, ident: syn::Ident, ty: Option<syn::Type>, kind: KoboBindingKind) {
        self.bindings.push(KoboBinding {
            id: self.id_gen.next_ast_id(),
            span: self.line_index.span_from_syn(ident.span(), self.file_id),
            ident,
            ty,
            kind,
        });
    }

    fn collect_binding_from_pat(&mut self, pat: &syn::Pat, kind: KoboBindingKind) {
        if let Some((ident, ty)) = binding_from_pat(pat) {
            self.push_binding(ident, ty, kind);
        }
    }
}

impl<'ast> Visit<'ast> for BindingCollector<'_> {
    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push_binding(
            node.ident.clone(),
            Some((*node.ty).clone()),
            KoboBindingKind::Const,
        );
        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push_binding(
            node.ident.clone(),
            Some((*node.ty).clone()),
            KoboBindingKind::Static,
        );
        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        for input in &node.sig.inputs {
            match input {
                syn::FnArg::Receiver(receiver) => {
                    self.collect_self_binding(receiver);
                }
                syn::FnArg::Typed(argument) => {
                    self.collect_parameter_binding(argument);
                }
            }
        }

        syn::visit::visit_block(self, &node.block);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        for input in &node.sig.inputs {
            match input {
                syn::FnArg::Receiver(receiver) => {
                    self.collect_self_binding(receiver);
                }
                syn::FnArg::Typed(argument) => {
                    self.collect_parameter_binding(argument);
                }
            }
        }

        syn::visit::visit_block(self, &node.block);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.collect_binding_from_pat(&node.pat, KoboBindingKind::Local);
        syn::visit::visit_local(self, node);
    }
}

impl BindingCollector<'_> {
    fn collect_parameter_binding(&mut self, argument: &syn::PatType) {
        let Some(ident) = binding_ident(&argument.pat) else {
            return;
        };

        self.push_binding(
            ident.clone(),
            Some((*argument.ty).clone()),
            KoboBindingKind::Parameter,
        );
    }

    fn collect_self_binding(&mut self, receiver: &syn::Receiver) {
        let kind = if receiver.mutability.is_some() {
            KoboBindingKind::MutSelfParam
        } else {
            KoboBindingKind::SelfParam
        };
        self.push_binding(
            syn::Ident::new("self", receiver.self_token.span),
            None,
            kind,
        );
    }
}

fn binding_from_pat(pat: &syn::Pat) -> Option<(syn::Ident, Option<syn::Type>)> {
    match pat {
        syn::Pat::Ident(ident) => Some((ident.ident.clone(), None)),
        syn::Pat::Type(typed) => {
            let (ident, _) = binding_from_pat(&typed.pat)?;
            Some((ident, Some((*typed.ty).clone())))
        }
        _ => None,
    }
}

fn binding_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}

struct LineIndex {
    line_starts: Vec<usize>,
    source_len: usize,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }

        Self {
            line_starts,
            source_len: source.len(),
        }
    }

    fn span_from_syn(&self, span: Span, file_id: FileId) -> KoboSpan {
        KoboSpan::new(
            self.byte_offset(span.start()) as u32,
            self.byte_offset(span.end()) as u32,
            file_id,
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use kobo_ir::FileId;

    use super::parse_file;
    use crate::KoboBindingKind;

    #[test]
    fn parse_assigns_unique_monotonic_ids() {
        let source = r#"
fn main() {
    let names = vec!["alice", "bob"];
    let count: i32 = 2;
    println!("{}", count);
}
"#;

        let mut id_gen = kobo_ir::NodeIdGen::new();
        let kobo_file = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");

        let mut all_ids = Vec::new();
        all_ids.extend(kobo_file.items.iter().map(|item| item.id.0));
        all_ids.extend(kobo_file.iter_bindings().map(|binding| binding.id.0));

        let unique_ids: HashSet<u32> = all_ids.iter().copied().collect();
        assert_eq!(all_ids.len(), unique_ids.len());
        assert!(all_ids.windows(2).all(|window| window[0] < window[1]));
    }

    #[test]
    fn test_binding_kind_self_param() {
        let source = r#"
struct Foo { val: i32 }
impl Foo {
    fn read(&self) -> i32 { self.val }
    fn write(&mut self, v: i32) { self.val = v; }
    fn consume(self) -> i32 { self.val }
    fn with_param(&self, x: i32) -> i32 { self.val + x }
}
"#;

        let mut id_gen = kobo_ir::NodeIdGen::new();
        let kobo_file = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");

        let bindings: Vec<_> = kobo_file.iter_bindings().collect();

        // read(&self) -> SelfParam
        let read_self = bindings
            .iter()
            .find(|b| b.ident == "self" && b.kind == KoboBindingKind::SelfParam);
        assert!(read_self.is_some(), "expected SelfParam for &self");

        // write(&mut self) -> MutSelfParam
        let write_self = bindings
            .iter()
            .find(|b| b.ident == "self" && b.kind == KoboBindingKind::MutSelfParam);
        assert!(write_self.is_some(), "expected MutSelfParam for &mut self");

        // consume(self) -> SelfParam (by-value self is not &mut)
        let consume_self = bindings
            .iter()
            .filter(|b| b.ident == "self" && b.kind == KoboBindingKind::SelfParam)
            .count();
        // read(&self), consume(self), and with_param(&self) are all SelfParam
        assert_eq!(
            consume_self, 3,
            "expected 3 SelfParam (read + consume + with_param)"
        );

        // with_param has x: i32 as Parameter
        let x_param = bindings
            .iter()
            .find(|b| b.ident == "x" && b.kind == KoboBindingKind::Parameter);
        assert!(x_param.is_some(), "expected Parameter for x");

        // v in write is Parameter
        let v_param = bindings
            .iter()
            .find(|b| b.ident == "v" && b.kind == KoboBindingKind::Parameter);
        assert!(v_param.is_some(), "expected Parameter for v");
    }
}
