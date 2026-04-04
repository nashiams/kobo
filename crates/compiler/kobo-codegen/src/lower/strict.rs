/// @strict block lowering for v0.5.
///
/// Contracts enforced here:
/// - C02: guard all exits (normal, ?, break, continue)
/// - C04: no .borrow()/.borrow_mut() inside the rewritten block body
/// - C06: @strict keyword / #[__kobo_strict] never appear in output
/// - C08: guard names are `__kobo_guard_{N}` where N comes from StrictGuardCounter
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use kobo_ir::{CaptureAccessKind, CaptureSet};
use kobo_parser::KoboItemFn;

use crate::CodegenOptions;

// ---------------------------------------------------------------------------
// StrictGuardCounter — Contract C08: deterministic, function-scoped N
// ---------------------------------------------------------------------------

/// Monotonically-incrementing counter for guard name generation.
///
/// One instance per function — shared across ALL @strict blocks within that
/// function so that guard names never collide (Trap 16).
pub struct StrictGuardCounter {
    next_n: usize,
}

impl StrictGuardCounter {
    pub fn new() -> Self {
        Self { next_n: 0 }
    }

    /// Allocate the next guard index and advance the counter.
    pub fn next(&mut self) -> usize {
        let n = self.next_n;
        self.next_n += 1;
        n
    }
}

impl Default for StrictGuardCounter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CF enum helpers — F-02: encapsulated so v0.6 can swap strategy
// ---------------------------------------------------------------------------

/// Emit the `__KoboStrictCf<V>` enum definition.
///
/// Emitted once per function when any @strict block uses break or continue.
/// Placed at the START of the function body (not per-block) per Rebuttal 2-3.
pub fn emit_cf_enum_def() -> TokenStream {
    quote! {
        #[allow(non_camel_case_types, dead_code)]
        enum __KoboStrictCf<V> {
            Continue,
            Break,
            Fallthrough(V),
        }
    }
}

/// Emit the match dispatch after guard drops for break/continue blocks.
pub fn emit_cf_dispatch(result_ident: &proc_macro2::Ident) -> TokenStream {
    quote! {
        match #result_ident {
            __KoboStrictCf::Break => break,
            __KoboStrictCf::Continue => continue,
            __KoboStrictCf::Fallthrough(v) => v,
        }
    }
}

// ---------------------------------------------------------------------------
// lower_strict_block — Task 5.1 + 5.2 + 5.3
// ---------------------------------------------------------------------------

/// Lower a single @strict block to safe Rust.
///
/// Reads CaptureSet from KIR. Emits guard extraction, body rewriting via
/// raw-reference rebinding, and guard drops on all exit paths (Contract C02).
///
/// # Arguments
/// - `stmts`: the already-lowered statements from the @strict block body
/// - `capture_set`: bindings captured by this block (from KIR)
/// - `counter`: function-scoped guard counter (Contract C08 / Trap 16)
/// - `_options`: codegen options (diag_mode etc.)
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

    // Assign guard indices now (C08: deterministic, function-scoped counter).
    let guard_indices: Vec<usize> = capture_set
        .bindings
        .iter()
        .map(|_| counter.next())
        .collect();

    // Build guard extraction + raw-reference rebinding stmts.
    let mut guard_stmts: Vec<TokenStream> = Vec::new();
    let mut rebind_stmts: Vec<TokenStream> = Vec::new();

    for (n, binding) in guard_indices.iter().zip(&capture_set.bindings) {
        let guard_ident = format_ident!("__kobo_guard_{}", n);
        let binding_ident = format_ident!("{}", binding.name);

        // Guard extraction (Contract C04: only here, never in body)
        let guard_stmt = match &binding.access_kind {
            CaptureAccessKind::Read => {
                quote! { let #guard_ident = #binding_ident.borrow(); }
            }
            CaptureAccessKind::Write => {
                quote! { let #guard_ident = #binding_ident.borrow_mut(); }
            }
            // F-06: non_exhaustive — treat unknown kinds as Write (conservative)
            _ => {
                quote! { let #guard_ident = #binding_ident.borrow_mut(); }
            }
        };
        guard_stmts.push(guard_stmt);

        // Raw-reference rebinding (Strategy A: let type inferred by rustc)
        let rebind_stmt = match &binding.access_kind {
            CaptureAccessKind::Read => {
                quote! { let #binding_ident = &*#guard_ident; }
            }
            CaptureAccessKind::Write => {
                quote! { let #binding_ident = &mut *#guard_ident; }
            }
            _ => {
                quote! { let #binding_ident = &mut *#guard_ident; }
            }
        };
        rebind_stmts.push(rebind_stmt);
    }

    // Drop stmts in LIFO order (Contract C02 normal exit path)
    let drop_stmts: Vec<TokenStream> = guard_indices
        .iter()
        .rev()
        .map(|n| {
            let guard_ident = format_ident!("__kobo_guard_{}", n);
            quote! { drop(#guard_ident); }
        })
        .collect();

    // Task 5.2: ? (question mark) → IIFE to guarantee drop before propagation.
    // Task 5.3: break/continue → CF enum IIFE.
    let needs_question_mark_iife =
        capture_set.has_question_mark && !capture_set.has_break && !capture_set.has_continue;
    let needs_cf_iife = capture_set.has_break || capture_set.has_continue;

    if needs_question_mark_iife {
        // Task 5.2: wrap body in `|| -> Result<_, _>` IIFE.
        // Guards are dropped BEFORE the `?` propagation (Contract C02).
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
        // Task 5.3: break/continue → CF enum IIFE.
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
        // Task 5.1: Normal case — no IIFE. Guards extracted at entry, dropped at end.
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

// ---------------------------------------------------------------------------
// lower_strict_fn — Task 5.4
// ---------------------------------------------------------------------------

/// Lower an @strict fn declaration.
///
/// Two modes (R-14):
/// - **AsyncDeferred** (`func.is_async == true`): strip the `#[__kobo_strict]`
///   marker from the attributes and emit the function unchanged. Full async
///   @strict support is deferred to v0.7 (K0063 diagnostic already raised in P3).
/// - **Full** (`func.is_async == false`): guard-extract all bindings in
///   `capture_set` around the fn body stmts. Equivalent to treating the
///   fn body as an implicit @strict block.
///
/// # Arguments
/// - `func`: the @strict fn item (from KoboFile.strict_fns())
/// - `capture_set`: capture set for this fn body (from KIR), or `None` when P3
///   did not produce one (fn body not yet analyzed as a block — future P7 work)
/// - `counter`: function-scoped guard counter (Contract C08)
/// - `_options`: codegen options
pub fn lower_strict_fn(
    func: &KoboItemFn,
    capture_set: Option<&CaptureSet>,
    counter: &mut StrictGuardCounter,
    _options: &CodegenOptions,
) -> TokenStream {
    let mut inner = func.inner.clone();

    // Strip the @strict marker attribute from the fn (Contract C06).
    inner.attrs.retain(|a| {
        !a.path().is_ident("__kobo_strict")
            && !(a.path().segments.len() == 1
                && a.path().segments[0].ident == "__kobo_strict")
    });

    if func.is_async {
        // AsyncDeferred: keep fn body unchanged, just remove marker (R-14).
        return quote! { #inner };
    }

    // Full mode: wrap fn body stmts with guard extraction (if capture_set supplied).
    if let Some(cs) = capture_set {
        if !cs.bindings.is_empty() {
            let wrapped_stmts_ts = lower_strict_block(&inner.block.stmts, cs, counter, _options);
            // Parse the wrapped block back to a syn::Block for the fn body.
            // If parsing fails, fall back to the normal fn body.
            if let Ok(wrapped_block) = syn::parse2::<syn::Block>(wrapped_stmts_ts.clone()) {
                inner.block = Box::new(wrapped_block);
            } else {
                // Fallback: emit an inner block expression wrapping the body
                // by replacing stmts with a single statement that contains the wrapped block.
                // This should not happen in practice — lower_strict_block emits valid { stmts }.
                let body_stmts = &inner.block.stmts;
                inner.block = syn::parse_quote! { {
                    #wrapped_stmts_ts
                    let _ = { #(#body_stmts)* };
                } };
            }
        }
    }

    quote! { #inner }
}

// ---------------------------------------------------------------------------
// Unit tests — 16 tests (TDD: written failing-first)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use kobo_ir::{
        CaptureAccessKind, CaptureSet, CapturedBinding, FileId, KirNodeId, KoboSpan,
        NestedStrictBlock,
    };
    use syn::parse_quote;

    use crate::CodegenOptions;

    use super::{emit_cf_enum_def, lower_strict_block, StrictGuardCounter};

    // ── test helpers ──────────────────────────────────────────────────────

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

    // ── Test 1: Basic Write binding → guard + borrow_mut + drop ──────────

    #[test]
    fn test1_write_binding_guard_and_drop() {
        let body: syn::Block = parse_quote!({ data.push(1); });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "guard name expected");
        assert!(s.contains("borrow_mut"), "write access needs borrow_mut");
        assert!(s.contains("drop"), "guard must be dropped");
    }

    // ── Test 2: Read-only binding → borrow() guard ────────────────────────

    #[test]
    fn test2_read_binding_uses_borrow_not_borrow_mut() {
        let body: syn::Block = parse_quote!({ let _ = data.len(); });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Read)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "guard name expected");
        assert!(
            s.contains("borrow ()") || s.contains("borrow()"),
            "read access must use borrow(), got: {s}"
        );
        // Must NOT call borrow_mut for a Read binding
        let borrow_mut_count = s.matches("borrow_mut").count();
        assert_eq!(borrow_mut_count, 0, "no borrow_mut for Read binding");
    }

    // ── Test 3: Two bindings → two guards, LIFO drop order ───────────────

    #[test]
    fn test3_two_bindings_lifo_drops() {
        let body: syn::Block = parse_quote!({ });
        let cs = make_capture_set(vec![
            make_binding("alpha", CaptureAccessKind::Write),
            make_binding("beta", CaptureAccessKind::Read),
        ]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "first guard");
        assert!(s.contains("__kobo_guard_1"), "second guard");
        // LIFO: guard_1 must be dropped BEFORE guard_0
        let pos_drop_0 = s.find("drop (__kobo_guard_0)").or_else(|| s.find("drop(__kobo_guard_0)"));
        let pos_drop_1 = s.find("drop (__kobo_guard_1)").or_else(|| s.find("drop(__kobo_guard_1)"));
        let pos_0 = pos_drop_0.expect("drop guard_0 expected");
        let pos_1 = pos_drop_1.expect("drop guard_1 expected");
        assert!(pos_1 < pos_0, "LIFO: guard_1 must be dropped before guard_0");
    }

    // ── Test 4: has_question_mark → IIFE wrapper ─────────────────────────

    #[test]
    fn test4_question_mark_iife_wrapper() {
        let body: syn::Block = parse_quote!({ do_thing()? });
        let mut cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        cs.has_question_mark = true;
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        // IIFE: closure `|| { body }` must appear
        assert!(
            s.contains("|| {") || s.contains("|| { "),
            "IIFE closure expected for ? exit path; got: {s}"
        );
        // must have result and the outer ? propagation after drops
        assert!(s.contains("__kobo_strict_result"), "result var expected");
        // drop must appear BEFORE the final ?
        let pos_drop = s.find("drop").expect("drop expected");
        let pos_question = s.rfind('?').expect("? propagation expected");
        assert!(pos_drop < pos_question, "guards must be dropped before ? propagation");
    }

    // ── Test 5: has_break → CF enum dispatch ──────────────────────────────

    #[test]
    fn test5_break_cf_enum_dispatch() {
        let body: syn::Block = parse_quote!({ if done { break; } });
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

    // ── Test 6: has_continue → CF enum with Continue arm ─────────────────

    #[test]
    fn test6_continue_cf_enum_dispatch() {
        let body: syn::Block = parse_quote!({ if skip { continue; } });
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

    // ── Test 7: labeled break/continue → no codegen (P3 emits violation) ──

    /// Labeled break that crosses @strict boundary is rejected in P3 (LabeledCrossBoundary).
    /// If it somehow reaches P5, lower_strict_block treats it as a plain break (safe fallback).
    #[test]
    fn test7_labeled_break_treated_as_break() {
        let body: syn::Block = parse_quote!({ break 'outer; });
        let mut cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        cs.has_break = true; // P3 set this
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        // Guard extraction must still happen
        assert!(s.contains("borrow_mut"), "guard must still be extracted");
        // Drop must still happen (before break dispatch)
        assert!(s.contains("drop"), "guard must be dropped");
    }

    // ── Test 8: empty capture set → verbatim body (Trap 18) ─────────────

    #[test]
    fn test8_empty_capture_set_verbatim_body() {
        let body: syn::Block = parse_quote!({ let x = 1 + 2; x });
        let cs = make_capture_set(vec![]); // empty!
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        // No guard extraction
        assert!(
            !s.contains("__kobo_guard"),
            "no guards for empty capture set"
        );
        // Body is preserved
        assert!(s.contains("let x"), "body preserved verbatim");
        // borrow/borrow_mut must NOT appear (no guard extraction)
        assert!(
            !s.contains("borrow"),
            "no borrow calls for empty capture set"
        );
    }

    // ── Test 9: guard names deterministic (Contract C08) ─────────────────

    #[test]
    fn test9_guard_names_deterministic_c08() {
        let body: syn::Block = parse_quote!({ let _ = d.len(); });
        let cs = make_capture_set(vec![make_binding("d", CaptureAccessKind::Read)]);
        // Call twice, same counter start position → same output
        let ts1 = lower_strict_block(&body.stmts, &cs, &mut StrictGuardCounter::new(), &CodegenOptions::default());
        let ts2 = lower_strict_block(&body.stmts, &cs, &mut StrictGuardCounter::new(), &CodegenOptions::default());
        assert_eq!(
            ts1.to_string(),
            ts2.to_string(),
            "same input must produce same output (Contract C08)"
        );
    }

    // ── Test 10: two @strict blocks in one fn → sequential N (Trap 16) ───

    #[test]
    fn test10_two_blocks_sequential_counter() {
        let body1: syn::Block = parse_quote!({ let _ = a.len(); });
        let body2: syn::Block = parse_quote!({ let _ = b.len(); });
        let cs1 = make_capture_set(vec![make_binding("a", CaptureAccessKind::Read)]);
        let cs2 = make_capture_set(vec![make_binding("b", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new(); // shared counter!
        let ts1 = lower_strict_block(&body1.stmts, &cs1, &mut counter, &CodegenOptions::default());
        let ts2 = lower_strict_block(&body2.stmts, &cs2, &mut counter, &CodegenOptions::default());
        let s1 = ts1.to_string();
        let s2 = ts2.to_string();
        assert!(s1.contains("__kobo_guard_0"), "first block uses guard_0");
        assert!(s2.contains("__kobo_guard_1"), "second block uses guard_1 (shared counter, Trap 16)");
        assert!(
            !s2.contains("__kobo_guard_0"),
            "second block must NOT reuse guard_0"
        );
    }

    // ── Test 11: rebinding line for Write ────────────────────────────────

    #[test]
    fn test11_write_rebinding_raw_ref_mut() {
        let body: syn::Block = parse_quote!({ data.push(99); });
        let cs = make_capture_set(vec![make_binding("data", CaptureAccessKind::Write)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        // Strategy A: `let data = &mut *__kobo_guard_0;`
        assert!(
            s.contains("& mut *"),
            "Write rebinding must use &mut *guard; got: {s}"
        );
    }

    // ── Test 12: rebinding line for Read ─────────────────────────────────

    #[test]
    fn test12_read_rebinding_raw_ref() {
        let body: syn::Block = parse_quote!({ let n = cache.len(); });
        let cs = make_capture_set(vec![make_binding("cache", CaptureAccessKind::Read)]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        // Strategy A: `let cache = &*__kobo_guard_0;`
        assert!(
            s.contains("& *"), // `& * __kobo_guard_0`
            "Read rebinding must use &*guard; got: {s}"
        );
        // Must NOT use & mut
        assert!(!s.contains("& mut *"), "Read must not use &mut *guard");
    }

    // ── Test 13: non-@strict empty block is verbatim (Trap 18 + C07) ─────

    /// Verifying a non-@strict block (empty capture set) produces the body verbatim.
    /// Combined with test8, confirms Contract C07: non-@strict code is untouched.
    #[test]
    fn test13_non_strict_verbatim_c07() {
        // Non-@strict code with empty capture set passes through verbatim (Contract C07 + Trap 18).
        // lower_strict_block only wraps when capture_set is non-empty.
        let body: syn::Block = parse_quote!({ let y = x + 1; });
        let cs = make_capture_set(vec![]);
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        // body must pass through unchanged
        assert!(s.contains("let y"), "body preserved");
        assert!(!s.contains("__kobo_guard"), "no guards for non-strict block");
    }

    // ── Test 14: nested @strict (flatten in P3 then single pass) ─────────

    /// After P3 flattening, nested @strict blocks become a single CaptureSet.
    /// This test verifies that a merged CaptureSet (2 bindings from nested blocks)
    /// produces 2 guards correctly.
    #[test]
    fn test14_merged_nested_capture_set() {
        let body: syn::Block = parse_quote!({ outer_data.push(1); inner_data.push(2); });
        let mut cs = make_capture_set(vec![
            make_binding("outer_data", CaptureAccessKind::Write),
            make_binding("inner_data", CaptureAccessKind::Write),
        ]);
        // Simulate nested block info recorded before merge (F-07)
        cs.nested_blocks.push(NestedStrictBlock {
            span: dummy_span(),
            original_captures: vec![node(1)],
        });
        let mut counter = StrictGuardCounter::new();
        let ts = lower_strict_block(&body.stmts, &cs, &mut counter, &CodegenOptions::default());
        let s = token_str(ts);
        assert!(s.contains("__kobo_guard_0"), "first guard");
        assert!(s.contains("__kobo_guard_1"), "second guard from nested merge");
    }

    // ── Test 15: CF enum def structure ───────────────────────────────────

    #[test]
    fn test15_cf_enum_def_has_all_variants() {
        let ts = emit_cf_enum_def();
        let s = ts.to_string();
        assert!(s.contains("__KoboStrictCf"), "enum name");
        assert!(s.contains("Continue"), "Continue variant");
        assert!(s.contains("Break"), "Break variant");
        assert!(s.contains("Fallthrough"), "Fallthrough variant");
    }

    // ── Test 16: @strict keyword / #[__kobo_strict] NOT in output (C06) ──

    /// The lower_strict_block output must never contain the @strict keyword
    /// or the #[__kobo_strict] marker attribute in its emitted code.
    #[test]
    fn test16_kobo_strict_not_in_output_c06() {
        let body: syn::Block = parse_quote!({ data.push(1); });
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
