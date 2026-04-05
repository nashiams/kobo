use crate::builder::build_transform_builder;
use crate::cfg::build_cfg;
use crate::finalize::{
    apply_validation_escalations, collect_hint_conflicts, finalize_transform,
    rewrite_dead_borrow_aliases,
};
use crate::options::TransformOptions;
use crate::strict::{analyze_strict_capture_set, flatten_nested_strict, validate_strict_boundary};
use crate::strict::span_convert::SpanConvert;
use crate::tier_validate::validate_tiers;
use crate::tiered::{apply_decisions, choose_tiers};
use crate::warn_early::detect_warn_early;
use kobo_ir::{Kir, NodeIdGen};
use kobo_parser::KoboFile;

/// Transforms a parsed Kobo file into the frozen KIR.
///
/// Keep this file as orchestration-only for the v0.7/v0.8 solver move:
/// 1. `build_transform_builder(...)` owns the AST walk and raw event recording.
/// 2. `finalize_transform(...)` seals the transform facts and borrow aliases.
/// 3. `choose_tiers(...)` is the only place that writes ownership into KIR.
/// 4. `validate_tiers(...)` may only rewrite decisions, then re-apply them
///    through the same tiered write path.
/// 5. `build_cfg(...)` stays after ownership is attached so later solver work
///    can lift `tiered.rs` out without re-threading the front half.
///
/// Future extraction path:
/// 1. Move `tiered.rs` to `crates/compiler/kobo-solve/src/greedy.rs`.
/// 2. Driver calls `kobo-solve` on frozen KIR instead of calling `tiered.rs`
///    inside transform.
/// 3. `kobo-transform` emits `OwnershipTier::Undecided` for all ownership
///    sites and the solver fills them in after KIR freeze.
pub fn build_kir(ast: &KoboFile, id_gen: &mut NodeIdGen, options: TransformOptions) -> Kir {
    let mut built = finalize_transform(build_transform_builder(ast, id_gen, options));
    let mut kir = Kir::from_nodes(built.nodes);
    rewrite_dead_borrow_aliases(&mut kir, &built.transform_facts, &built.borrow_aliases);

    let mut decisions = choose_tiers(&built.transform_facts, &mut kir);
    let violations = validate_tiers(&decisions, &built.transform_facts);
    apply_validation_escalations(&mut decisions, &violations);
    if !violations.is_empty() {
        apply_decisions(&mut kir, &decisions);
    }

    collect_hint_conflicts(&mut built.transform_facts, &decisions);
    kir.set_struct_defs(built.struct_defs);
    kir.set_transform_facts(built.transform_facts.clone());
    kir.set_tier_decisions(decisions);

    let warn_early = detect_warn_early(&kir);
    kir.set_warn_early_facts(warn_early);

    // v0.5: @strict analysis, AFTER finalized TierDecisions (R-02).
    // Pipeline sequence:
    // 3a. For each @strict block: analyze_strict_capture_set()
    // 3b. Nested @strict blocks within the same fn: flatten_nested_strict()
    // 3c. For each @strict block: validate_strict_boundary()
    // 4. Store results in KIR.
    //
    // `ast.strict_blocks()` is populated by `collect_strict_items_from_syn`
    // (called by the driver / test harness) BEFORE `postprocess_strict_markers`
    // strips the `#[__kobo_strict]` attributes.
    {
        let transform_facts = kir.transform_facts().clone();
        let (mut capture_sets, mut boundary_facts) = (Vec::new(), Vec::new());
        let sc = SpanConvert::from_kobo_file(ast);

        // @strict async fns: K0063 — skip analysis, deferred to v0.7.
        // (The warning is emitted by the analysis phase, not here.)

        for kblock in ast.strict_blocks() {
            let mut cap =
                analyze_strict_capture_set(kblock, &transform_facts, &kir, &sc);

            // Collect nested @strict blocks within this block and flatten (R-12).
            let nested: Vec<_> = ast
                .strict_blocks()
                .iter()
                .filter(|nb| {
                    nb.span.start > kblock.span.start
                        && nb.span.end < kblock.span.end
                })
                .map(|nb| analyze_strict_capture_set(nb, &transform_facts, &kir, &sc))
                .collect();
            flatten_nested_strict(&mut cap, nested);

            // Find enclosing function body for K0041 alias detection.
            let enclosing_stmts = find_enclosing_fn_stmts(&ast.inner, kblock.span, &sc);
            let mut facts = validate_strict_boundary(
                kblock,
                &cap,
                &transform_facts,
                &kir,
                &enclosing_stmts,
                &sc,
            );
            capture_sets.push(cap);
            boundary_facts.append(&mut facts);
        }

        kir.set_strict_capture_sets(capture_sets);
        kir.set_strict_boundary_facts(boundary_facts);
    }

    // Build strict_fn_modes map for all @strict fns.
    {
        use kobo_ir::StrictFnMode;
        let mut fn_modes = std::collections::HashMap::new();
        for func in ast.strict_fns() {
            let mode = if func.is_async {
                StrictFnMode::AsyncDeferred
            } else {
                StrictFnMode::Full
            };
            fn_modes.insert(func.span, mode);
        }
        kir.set_strict_fn_modes(fn_modes);
    }

    let _cfg = build_cfg(&kir);
    kir
}

/// Find the statements of the enclosing function body for a given span.
///
/// Walks top-level items looking for `fn` items whose span contains `block_span`.
/// Returns the function body statements, or an empty vec if no enclosing fn found.
fn find_enclosing_fn_stmts(
    file: &syn::File,
    block_span: kobo_ir::KoboSpan,
    sc: &SpanConvert,
) -> Vec<syn::Stmt> {
    use syn::Item;
    use syn::spanned::Spanned;

    for item in &file.items {
        if let Item::Fn(item_fn) = item {
            let fn_span = sc.span(item_fn.span());
            if fn_span.file_id == block_span.file_id
                && fn_span.start <= block_span.start
                && block_span.end <= fn_span.end
            {
                return item_fn.block.stmts.clone();
            }
        }
    }
    Vec::new()
}
