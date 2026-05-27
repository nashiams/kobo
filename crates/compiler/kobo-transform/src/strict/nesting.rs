/// Nested @strict block flattening.
///
/// Merge rule: max(Read, Write) = Write.
use kobo_ir::{CaptureAccessKind, CaptureSet, CapturedBinding, NestedStrictBlock};

/// Merge inner @strict captures into the outer capture set.
///
/// Merge rule: max(Read, Write) = Write.
/// Any mutable path in any nested block escalates the outer capture to Write.
///
/// Records nested block info in CaptureSet.nested_blocks before merging so
/// later passes can recover per-block granularity.
pub fn flatten_nested_strict(outer: &mut CaptureSet, inner_blocks: Vec<CaptureSet>) {
    for inner in inner_blocks {
        outer.nested_blocks.push(NestedStrictBlock {
            span: inner.block_span,
            original_captures: inner.bindings.iter().map(|b| b.binding_id).collect(),
        });

        for inner_binding in inner.bindings {
            merge_binding(outer, inner_binding);
        }

        outer.has_question_mark |= inner.has_question_mark;
        outer.has_break |= inner.has_break;
        outer.has_continue |= inner.has_continue;
        outer.is_inside_loop |= inner.is_inside_loop;
    }
}

/// Merge one inner binding into the outer capture set.
/// If the binding already exists: max(Read, Write) = Write.
/// If it's new: add it.
fn merge_binding(outer: &mut CaptureSet, inner: CapturedBinding) {
    if let Some(existing) = outer
        .bindings
        .iter_mut()
        .find(|b| b.binding_id == inner.binding_id)
    {
        // Write escalates Read when nested blocks share a binding.
        if inner.access_kind == CaptureAccessKind::Write {
            existing.access_kind = CaptureAccessKind::Write;
        }
        existing.access_count += inner.access_count;
        existing.access_spans.extend(inner.access_spans);
    } else {
        outer.bindings.push(inner);
    }
}
