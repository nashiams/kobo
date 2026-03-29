use crate::builder::build_transform_builder;
use crate::cfg::build_cfg;
use crate::finalize::{
    apply_validation_escalations, collect_hint_conflicts, finalize_transform,
    rewrite_dead_borrow_aliases,
};
use crate::options::TransformOptions;
use crate::tier_validate::validate_tiers;
use crate::tiered::{apply_decisions, choose_tiers};
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
    kir.set_transform_facts(built.transform_facts);
    kir.set_tier_decisions(decisions);

    let _cfg = build_cfg(&kir);
    kir
}
