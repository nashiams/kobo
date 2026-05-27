/// Lower a single @strict block to safe Rust.
///
/// Task 5.1 + 5.2 + 5.3.
/// Contracts enforced: C02 (guard all exits), C04 (no borrow in body),
/// C06 (@strict never in output), C08 (deterministic guard names).
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use kobo_ir::{CaptureAccessKind, CaptureSet};

use super::cf::emit_cf_dispatch;
use super::guard::StrictGuardCounter;
use crate::CodegenOptions;

/// Lower a single @strict block to safe Rust.
///
/// Reads CaptureSet from KIR. Emits guard extraction, body rewriting via
/// raw-reference rebinding, and guard drops on all exit paths (Invariant C02).
pub fn lower_strict_block(
    stmts: &[syn::Stmt],
    capture_set: &CaptureSet,
    counter: &mut StrictGuardCounter,
    _options: &CodegenOptions,
) -> TokenStream {
    // Trap 18: empty capture set → pass through block body verbatim, no guards.
    if capture_set.bindings.is_empty() {
        return quote! { { #(#stmts)* } };
    }

    let guard_indices: Vec<usize> = capture_set
        .bindings
        .iter()
        .map(|_| counter.next())
        .collect();

    let guard_stmts = build_guard_stmts(&guard_indices, capture_set);
    let rebind_stmts = build_rebind_stmts(&guard_indices, capture_set);
    let drop_stmts = build_drop_stmts(&guard_indices);

    emit_block_body(stmts, capture_set, &guard_stmts, &rebind_stmts, &drop_stmts)
}

fn build_guard_stmts(guard_indices: &[usize], capture_set: &CaptureSet) -> Vec<TokenStream> {
    guard_indices
        .iter()
        .zip(&capture_set.bindings)
        .map(|(n, binding)| {
            let guard_ident = format_ident!("__kobo_guard_{}", n);
            let binding_ident = format_ident!("{}", binding.name);
            match &binding.access_kind {
                CaptureAccessKind::Read => {
                    quote! { let #guard_ident = #binding_ident.borrow(); }
                }
                CaptureAccessKind::Write => {
                    quote! { let #guard_ident = #binding_ident.borrow_mut(); }
                }
                _ => {
                    quote! { let #guard_ident = #binding_ident.borrow_mut(); }
                }
            }
        })
        .collect()
}

fn build_rebind_stmts(guard_indices: &[usize], capture_set: &CaptureSet) -> Vec<TokenStream> {
    guard_indices
        .iter()
        .zip(&capture_set.bindings)
        .map(|(n, binding)| {
            let guard_ident = format_ident!("__kobo_guard_{}", n);
            let binding_ident = format_ident!("{}", binding.name);
            match &binding.access_kind {
                CaptureAccessKind::Read => {
                    quote! { let #binding_ident = &*#guard_ident; }
                }
                CaptureAccessKind::Write => {
                    quote! { let #binding_ident = &mut *#guard_ident; }
                }
                _ => {
                    quote! { let #binding_ident = &mut *#guard_ident; }
                }
            }
        })
        .collect()
}

/// Drop stmts in LIFO order (Invariant C02 normal exit path).
fn build_drop_stmts(guard_indices: &[usize]) -> Vec<TokenStream> {
    guard_indices
        .iter()
        .rev()
        .map(|n| {
            let guard_ident = format_ident!("__kobo_guard_{}", n);
            quote! { drop(#guard_ident); }
        })
        .collect()
}

fn emit_block_body(
    stmts: &[syn::Stmt],
    capture_set: &CaptureSet,
    guard_stmts: &[TokenStream],
    rebind_stmts: &[TokenStream],
    drop_stmts: &[TokenStream],
) -> TokenStream {
    let needs_question_mark_iife =
        capture_set.has_question_mark && !capture_set.has_break && !capture_set.has_continue;
    let needs_cf_iife = capture_set.has_break || capture_set.has_continue;

    if needs_question_mark_iife {
        quote! {
            {
                #(#guard_stmts)*
                #(#rebind_stmts)*
                // kobo: @strict entry — guard-wrapped closure for ? propagation (C02)
                let __kobo_strict_result = (|| { #(#stmts)* })();
                #(#drop_stmts)*
                __kobo_strict_result?
            }
        }
    } else if needs_cf_iife {
        let cf_result_ident = format_ident!("__kobo_strict_cf");
        let dispatch = emit_cf_dispatch(&cf_result_ident);
        quote! {
            {
                #(#guard_stmts)*
                #(#rebind_stmts)*
                // kobo: @strict entry — CF enum closure for break/continue (C02)
                let #cf_result_ident = (|| {
                    #(#stmts)*
                    __KoboStrictCf::Fallthrough(())
                })();
                #(#drop_stmts)*
                #dispatch
            }
        }
    } else {
        quote! {
            {
                // kobo: @strict entry — borrow extracted
                #(#guard_stmts)*
                #(#rebind_stmts)*
                #(#stmts)*
                // kobo: @strict exit — guards dropped (LIFO)
                #(#drop_stmts)*
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use kobo_ir::{
        CaptureAccessKind, CaptureSet, CapturedBinding, FileId, KirNodeId, KoboSpan,
        NestedStrictBlock,
    };
    use syn::parse_quote;

    use crate::CodegenOptions;

    use super::super::cf::emit_cf_enum_def;
    use super::{lower_strict_block, StrictGuardCounter};

    fn dummy_span() -> KoboSpan {
        KoboSpan::new(0, 1, FileId(0))
    }

    fn node(id: u32) -> KirNodeId {
        KirNodeId(id)
    }

    fn make_binding(name: &str, kind: CaptureAccessKind) -> CapturedBinding {
        CapturedBinding {
            binding_id: node(0),
            name: name.to_owned(),
            access_kind: kind,
            access_count: 1,
            access_spans: vec![],
        }
    }

    fn make_capture_set(bindings: Vec<CapturedBinding>) -> CaptureSet {
        CaptureSet {
            block_span: dummy_span(),
            bindings,
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        }
    }

    fn token_str(ts: proc_macro2::TokenStream) -> String {
        ts.to_string()
    }

    #[test]
    fn test1_write_binding_guard_and_drop() {
        let body: syn::Block = parse_quote!({
            data.push(1);
        });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "guard name expected");
        assert!(s.contains("borrow_mut"), "write access needs borrow_mut");
        assert!(s.contains("drop"), "guard must be dropped");
    }

    #[test]
    fn test2_read_binding_uses_borrow_not_borrow_mut() {
        let body: syn::Block = parse_quote!({
            let _ = data.len();
        });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Read)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "guard name expected");
        assert!(
            s.contains("borrow ()") || s.contains("borrow()"),
            "read access must use borrow(), got: {s}"
        );
        let borrow_mut_count = s.matches("borrow_mut").count();
        assert_eq!(borrow_mut_count, 0, "no borrow_mut for Read binding");
    }

    #[test]
    fn test3_two_bindings_lifo_drops() {
        let body: syn::Block = parse_quote!({});
        let cs = make_capture_set(vec![
            make_binding("alpha", CaptureAccessKind::Write),
            make_binding("beta", CaptureAccessKind::Read),
        ]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "first guard");
        assert!(s.contains("__kobo_guard_1"), "second guard");
        let pos_drop_0 = s
            .find("drop (__kobo_guard_0)")
            .or_else(|| s.find("drop(__kobo_guard_0)"));
        let pos_drop_1 = s
            .find("drop (__kobo_guard_1)")
            .or_else(|| s.find("drop(__kobo_guard_1)"));
        let pos_0 = pos_drop_0.expect("drop guard_0 expected");
        let pos_1 = pos_drop_1.expect("drop guard_1 expected");
        assert!(
            pos_1 < pos_0,
            "LIFO: guard_1 must be dropped before guard_0"
        );
    }

    #[test]
    fn test4_question_mark_iife_wrapper() {
        let body: syn::Block = parse_quote!({ do_thing()? });
        let mut cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        cs.has_question_mark = true;
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            s.contains("|| {") || s.contains("|| { "),
            "IIFE closure expected for ? exit path; got: {s}"
        );
        assert!(s.contains("__kobo_strict_result"), "result var expected");
        let pos_drop = s.find("drop").expect("drop expected");
        let pos_question = s.rfind('?').expect("? propagation expected");
        assert!(
            pos_drop < pos_question,
            "guards must be dropped before ? propagation"
        );
    }

    #[test]
    fn test5_break_cf_enum_dispatch() {
        let body: syn::Block = parse_quote!({
            if done {
                break;
            }
        });
        let mut cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        cs.has_break = true;
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            s.contains("__KoboStrictCf"),
            "CF enum dispatch expected for break; got: {s}"
        );
        assert!(s.contains("break"), "break arm expected");
    }

    #[test]
    fn test6_continue_cf_enum_dispatch() {
        let body: syn::Block = parse_quote!({
            if skip {
                continue;
            }
        });
        let mut cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        cs.has_continue = true;
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            s.contains("__KoboStrictCf"),
            "CF enum dispatch expected for continue; got: {s}"
        );
        assert!(s.contains("continue"), "continue arm expected");
    }

    #[test]
    fn test7_labeled_break_treated_as_break() {
        let body: syn::Block = parse_quote!({
            break 'outer;
        });
        let mut cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        cs.has_break = true;
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("borrow_mut"), "guard must still be extracted");
        assert!(s.contains("drop"), "guard must be dropped");
    }

    #[test]
    fn test8_empty_capture_set_verbatim_body() {
        let body: syn::Block = parse_quote!({
            let x = 1 + 2;
            x
        });
        let cs = make_capture_set(vec![]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            !s.contains("__kobo_guard"),
            "no guards for empty capture set"
        );
        assert!(s.contains("let x"), "body preserved verbatim");
        assert!(
            !s.contains("borrow"),
            "no borrow calls for empty capture set"
        );
    }

    #[test]
    fn test9_guard_names_deterministic_c08() {
        let body: syn::Block = parse_quote!({
            let _ = d.len();
        });
        let cs = make_capture_set(vec![make_binding("d", CaptureAccessKind::Read)]);
        let ts1 = lower_strict_block(
            &body.stmts,
            &cs,
            &mut StrictGuardCounter::new(),
            &CodegenOptions::default(),
        );
        let ts2 = lower_strict_block(
            &body.stmts,
            &cs,
            &mut StrictGuardCounter::new(),
            &CodegenOptions::default(),
        );
        assert_eq!(
            ts1.to_string(),
            ts2.to_string(),
            "same input must produce same output (Invariant C08)"
        );
    }

    #[test]
    fn test10_two_blocks_sequential_counter() {
        let body1: syn::Block = parse_quote!({
            let _ = a.len();
        });
        let body2: syn::Block = parse_quote!({
            let _ = b.len();
        });
        let cs1 = make_capture_set(vec![make_binding("a", CaptureAccessKind::Read)]);
        let cs2 = make_capture_set(vec![make_binding("b", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new();
        let ts1 = lower_strict_block(&body1.stmts, &cs1, &mut counter, &CodegenOptions::default());
        let ts2 = lower_strict_block(&body2.stmts, &cs2, &mut counter, &CodegenOptions::default());
        let s1 = ts1.to_string();
        let s2 = ts2.to_string();
        assert!(s1.contains("__kobo_guard_0"), "first block uses guard_0");
        assert!(
            s2.contains("__kobo_guard_1"),
            "second block uses guard_1 (shared counter, Trap 16)"
        );
        assert!(
            !s2.contains("__kobo_guard_0"),
            "second block must NOT reuse guard_0"
        );
    }

    #[test]
    fn test11_write_rebinding_raw_ref_mut() {
        let body: syn::Block = parse_quote!({
            data.push(99);
        });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            s.contains("& mut *"),
            "Write rebinding must use &mut *guard; got: {s}"
        );
    }

    #[test]
    fn test12_read_rebinding_raw_ref() {
        let body: syn::Block = parse_quote!({
            let n = cache.len();
        });
        let cs = make_capture_set(vec![make_binding("cache", CaptureAccessKind::Read)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            s.contains("& *"),
            "Read rebinding must use &*guard; got: {s}"
        );
        assert!(!s.contains("& mut *"), "Read must not use &mut *guard");
    }

    #[test]
    fn test13_non_strict_verbatim_c07() {
        let body: syn::Block = parse_quote!({
            let y = x + 1;
        });
        let cs = make_capture_set(vec![]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("let y"), "body preserved");
        assert!(
            !s.contains("__kobo_guard"),
            "no guards for non-strict block"
        );
    }

    #[test]
    fn test14_merged_nested_capture_set() {
        let body: syn::Block = parse_quote!({
            outer_data.push(1);
            inner_data.push(2);
        });
        let mut cs = make_capture_set(vec![
            make_binding("outer_data", CaptureAccessKind::Write),
            make_binding("inner_data", CaptureAccessKind::Write),
        ]);
        cs.nested_blocks.push(NestedStrictBlock {
            span: dummy_span(),
            original_captures: vec![node(1)],
        });
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "first guard");
        assert!(
            s.contains("__kobo_guard_1"),
            "second guard from nested merge"
        );
    }

    #[test]
    fn test15_cf_enum_def_has_all_variants() {
        let ts = emit_cf_enum_def();
        let s = ts.to_string();
        assert!(s.contains("__KoboStrictCf"), "enum name");
        assert!(s.contains("Continue"), "Continue variant");
        assert!(s.contains("Break"), "Break variant");
        assert!(s.contains("Fallthrough"), "Fallthrough variant");
    }

    #[test]
    fn test16_kobo_strict_not_in_output_c06() {
        let body: syn::Block = parse_quote!({
            data.push(1);
        });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(
            !s.contains("@strict"),
            "@strict keyword must not appear in output (C06)"
        );
        assert!(
            !s.contains("__kobo_strict"),
            "#[__kobo_strict] marker must not appear in output (C06)"
        );
    }
}
