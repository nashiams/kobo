use super::*;

impl<'a> Lowerer<'a> {
    pub(super) fn record_binding_anchor(
        &mut self,
        binding: &kobo_parser::KoboBinding,
        kind: LoweringAnchorKind,
    ) {
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        self.anchors.push(LoweringAnchor { node, kind });
    }

    pub(super) fn record_item_anchor(&mut self, ident: &syn::Ident, kind: LoweringAnchorKind) {
        let span = self.ast.span_from_syn(ident.span());
        let Some(binding) = self.ast.binding_for_span(span) else {
            return;
        };
        self.record_binding_anchor(binding, kind);
    }

    pub(super) fn source_line_for_expr_for_loop(&self, for_loop: &syn::ExprForLoop) -> usize {
        let span = self.ast.span_from_syn(for_loop.for_token.span);
        let (line, _) = self.ast.line_col(span);
        line
    }

    pub(super) fn source_line_for_macro(&self, mac: &syn::Macro) -> usize {
        let span = self.ast.span_from_syn(mac.path.span());
        let (line, _) = self.ast.line_col(span);
        line
    }
}
