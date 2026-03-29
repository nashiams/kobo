use std::collections::HashMap;

use kobo_ir::KirNodeId;
use syn::visit::{self, Visit};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LoweringAnchorKind {
    Parameter,
    Local,
    Const,
    Static,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FormattedAnchor {
    kind: LoweringAnchorKind,
    location: ResolvedAnchor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedAnchor {
    pub(crate) line: usize,
    pub(crate) column_start: usize,
    pub(crate) column_end: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedAnchorMap {
    by_node: HashMap<KirNodeId, ResolvedAnchor>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LoweringAnchorMap {
    anchors: Vec<LoweringAnchor>,
    support_item_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoweringAnchor {
    pub(crate) node: KirNodeId,
    pub(crate) kind: LoweringAnchorKind,
}

impl LoweringAnchorMap {
    pub(crate) fn new(anchors: Vec<LoweringAnchor>, support_item_count: usize) -> Self {
        Self {
            anchors,
            support_item_count,
        }
    }

    pub(crate) fn resolve(&self, formatted: &str) -> ResolvedAnchorMap {
        let parsed = syn::parse_file(formatted)
            .unwrap_or_else(|_| unreachable!("invariant: emitted Rust should always parse"));
        let formatted_anchors =
            FormattedBindingCollector::collect(&parsed, self.support_item_count);
        debug_assert_eq!(self.anchors.len(), formatted_anchors.len());

        let mut by_node = HashMap::with_capacity(self.anchors.len());
        for (source_anchor, formatted_anchor) in self.anchors.iter().zip(formatted_anchors) {
            debug_assert_eq!(source_anchor.kind, formatted_anchor.kind);
            by_node.insert(source_anchor.node, formatted_anchor.location);
        }

        ResolvedAnchorMap { by_node }
    }
}

impl ResolvedAnchorMap {
    pub(crate) fn get(&self, node: KirNodeId) -> Option<ResolvedAnchor> {
        self.by_node.get(&node).copied()
    }

    #[cfg(test)]
    pub(crate) fn insert(&mut self, node: KirNodeId, anchor: ResolvedAnchor) {
        self.by_node.insert(node, anchor);
    }
}

#[derive(Default)]
struct FormattedBindingCollector {
    anchors: Vec<FormattedAnchor>,
}

impl FormattedBindingCollector {
    fn collect(file: &syn::File, support_item_count: usize) -> Vec<FormattedAnchor> {
        let mut collector = Self::default();
        for item in file.items.iter().skip(support_item_count) {
            collector.visit_item(item);
        }
        collector.anchors
    }

    fn push_binding(&mut self, ident: &syn::Ident, kind: LoweringAnchorKind) {
        let start = ident.span().start();
        let end = ident.span().end();
        self.anchors.push(FormattedAnchor {
            kind,
            location: ResolvedAnchor {
                line: start.line,
                column_start: start.column,
                column_end: end.column.max(start.column + 1),
            },
        });
    }
}

impl<'ast> Visit<'ast> for FormattedBindingCollector {
    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        self.push_binding(&node.ident, LoweringAnchorKind::Const);
        visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        self.push_binding(&node.ident, LoweringAnchorKind::Static);
        visit::visit_item_static(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        for input in &node.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                if let Some(ident) = binding_ident(&argument.pat) {
                    self.push_binding(ident, LoweringAnchorKind::Parameter);
                }
            }
        }

        visit::visit_block(self, &node.block);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some(ident) = binding_ident(&node.pat) {
            self.push_binding(ident, LoweringAnchorKind::Local);
        }
        visit::visit_local(self, node);
    }
}

fn binding_ident(pat: &syn::Pat) -> Option<&syn::Ident> {
    match pat {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}
