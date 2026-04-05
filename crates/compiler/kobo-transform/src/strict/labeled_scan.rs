/// Detect labeled break/continue crossing @strict boundary (Trap 22, R-13).
///
/// Heuristic: any labeled break/continue inside the block is flagged
/// (conservative; exact label resolution requires CFG).
use kobo_ir::{CaptureSet, KoboSpan, StrictBoundaryFact, StrictBoundaryViolation};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

use super::span_convert::SpanConvert;

pub(super) fn check_labeled_cross_boundary(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    facts: &mut Vec<StrictBoundaryFact>,
    sc: &SpanConvert,
) {
    let mut visitor = LabeledBreakVisitor::new(sc);
    visitor.visit_block(&block.body);

    for (label, break_span) in visitor.labeled_breaks {
        facts.push(StrictBoundaryFact {
            block_span: capture_set.block_span,
            violation: StrictBoundaryViolation::LabeledCrossBoundary {
                label: label.clone(),
                break_or_continue_span: break_span,
                target_label_span: break_span,
            },
        });
    }
}

// --- LabeledBreakVisitor ---

struct LabeledBreakVisitor<'a> {
    labeled_breaks: Vec<(String, KoboSpan)>,
    inside_loop_depth: u32,
    sc: &'a SpanConvert,
}

impl<'a> LabeledBreakVisitor<'a> {
    fn new(sc: &'a SpanConvert) -> Self {
        Self {
            labeled_breaks: Vec::new(),
            inside_loop_depth: 0,
            sc,
        }
    }
}

impl<'a, 'ast> Visit<'ast> for LabeledBreakVisitor<'a> {
    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.inside_loop_depth += 1;
        syn::visit::visit_expr_loop(self, node);
        self.inside_loop_depth -= 1;
    }

    fn visit_expr_break(&mut self, node: &'ast syn::ExprBreak) {
        if let Some(label) = &node.label {
            let label_name = label.ident.to_string();
            use syn::spanned::Spanned;
            let break_span = self.sc.span(node.span());
            self.labeled_breaks.push((label_name, break_span));
        }
        syn::visit::visit_expr_break(self, node);
    }

    fn visit_expr_continue(&mut self, node: &'ast syn::ExprContinue) {
        if let Some(label) = &node.label {
            let label_name = label.ident.to_string();
            use syn::spanned::Spanned;
            let break_span = self.sc.span(node.span());
            self.labeled_breaks.push((label_name, break_span));
        }
        syn::visit::visit_expr_continue(self, node);
    }
}
