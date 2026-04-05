/// P3 end-to-end integration tests for @strict block analysis.
///
/// Written FAILING first (TDD). These tests verify that `build_kir` correctly
/// populates `kir.strict_capture_sets()` and `kir.strict_boundary_facts()`
/// when the caller provides a properly-preprocessed `KoboFile`.
///
/// Failing state (before step 5): `strict_blocks()` is empty because
/// `collect_strict_items_from_syn` is not yet wired in the parse helper,
/// so `build_kir` produces zero capture sets.
///
/// Passing state (after step 5): the full pipeline helper is used.
#[cfg(test)]
mod tests {
    use kobo_ir::FileId;
    use kobo_parser::{parse_file, KoboFile};

    use crate::options::TransformOptions;
    use crate::transform::build_kir;

    // ── Test pipeline helpers ──────────────────────────────────────────────

    /// Parse @strict source WITHOUT wiring strict-item collection.
    /// Used to write FAILING tests first (TDD step 1).
    fn parse_preprocess_only(source: &str) -> (KoboFile, kobo_ir::NodeIdGen) {
        use kobo_parser::{
            postprocess_strict_markers, preprocess_kobo_keywords, v05_keyword_configs,
        };
        let configs = v05_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        let file_id = FileId(0);
        let mut id_gen = kobo_ir::NodeIdGen::new();
        let mut kobo_file = parse_file(&rewritten, file_id, &mut id_gen).expect("parse ok");
        // ❌ NOT calling collect_strict_items_from_syn here intentionally.
        //    This is the failing-first state.
        postprocess_strict_markers(&mut kobo_file.inner, &markers).expect("postprocess ok");
        (kobo_file, id_gen)
    }

    /// Full pipeline: preprocess → parse → collect strict items → strip markers.
    /// Tests use this helper after wiring (TDD step 2 — make tests pass).
    fn parse_kobo_strict(source: &str) -> (KoboFile, kobo_ir::NodeIdGen) {
        use kobo_parser::{
            collect_strict_items_from_syn, postprocess_strict_markers,
            preprocess_kobo_keywords, v05_keyword_configs,
        };
        let configs = v05_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        let file_id = FileId(0);
        let mut id_gen = kobo_ir::NodeIdGen::new();
        let mut kobo_file = parse_file(&rewritten, file_id, &mut id_gen).expect("parse ok");
        let (strict_blocks, strict_fns) =
            collect_strict_items_from_syn(&kobo_file.inner, &rewritten, file_id);
        kobo_file.set_strict_items(strict_blocks, strict_fns);
        postprocess_strict_markers(&mut kobo_file.inner, &markers).expect("postprocess ok");
        (kobo_file, id_gen)
    }

    // ── Integration test 1: empty @strict block ────────────────────────────

    /// After the full pipeline, `build_kir` should produce exactly one
    /// capture-set entry for one @strict block (even if the block is empty).
    #[test]
    fn test_p3_empty_strict_block_produces_one_capture_set() {
        let source = "fn f() { @strict { } }";
        let (ast, mut id_gen) = parse_kobo_strict(source);
        assert_eq!(ast.strict_blocks().len(), 1, "parser must collect 1 @strict block");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        assert_eq!(
            kir.strict_capture_sets().len(),
            1,
            "build_kir must produce 1 capture set for 1 @strict block"
        );
        assert!(
            kir.strict_capture_sets()[0].bindings.is_empty(),
            "empty @strict block has no captured bindings"
        );
    }

    // ── Integration test 2: no @strict → no capture sets ──────────────────

    /// Without any @strict blocks, `strict_capture_sets` must be empty.
    ///
    /// Tests both the pipeline helper AND the no-preprocess path — they should
    /// agree since there are no @strict markers.
    #[test]
    fn test_p3_no_strict_source_empty_capture_sets() {
        let source = "fn f() { let x: i32 = 1; }";
        let (ast, mut id_gen) = parse_kobo_strict(source);
        assert_eq!(ast.strict_blocks().len(), 0, "no @strict blocks in plain source");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        assert!(
            kir.strict_capture_sets().is_empty(),
            "no capture sets when source has no @strict blocks"
        );
    }

    // ── Integration test 3: strict_blocks match block count ───────────────

    /// Multiple @strict blocks in source → matching number of strict_blocks.
    #[test]
    fn test_p3_two_strict_blocks_two_capture_sets() {
        let source = r#"
fn f() {
    @strict { let a: i32 = 1; }
    @strict { let b: i32 = 2; }
}
"#;
        let (ast, mut id_gen) = parse_kobo_strict(source);
        assert_eq!(
            ast.strict_blocks().len(),
            2,
            "parser must collect 2 @strict blocks"
        );
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        assert_eq!(
            kir.strict_capture_sets().len(),
            2,
            "build_kir must produce 2 capture sets for 2 @strict blocks"
        );
    }

    // ── Integration test 4: without full pipeline → no capture sets ────────

    /// Verifies that when the caller uses ONLY preprocessing (not collection),
    /// `build_kir` produces zero capture sets — confirming the integration
    /// truly depends on `collect_strict_items_from_syn` being called.
    #[test]
    fn test_p3_preprocess_only_no_capture_sets() {
        let source = "fn f() { @strict { } }";
        let (ast, mut id_gen) = parse_preprocess_only(source);
        assert_eq!(
            ast.strict_blocks().len(),
            0,
            "without collect_strict_items_from_syn, strict_blocks must be empty"
        );
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        assert!(
            kir.strict_capture_sets().is_empty(),
            "without strict_blocks, build_kir produces no capture sets"
        );
    }
}
