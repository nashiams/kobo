/// P1 parser tests for @strict syntax recognition.
///
/// Written FAILING first (TDD). Implement preprocess.rs and ast extensions
/// to make these pass.
#[cfg(test)]
mod tests {
    use crate::preprocess::{preprocess_kobo_keywords, strict_keyword_configs};

    // Test 1: @strict { body } → preprocessed source contains #[__kobo_strict]
    // and parse_file succeeds; is_strict flag is tracked by preprocessor.
    #[test]
    fn test_strict_block_preprocessed() {
        let source = "fn f() { @strict { let x = 1; } }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(!markers.is_empty(), "should find @strict marker");
        assert!(
            rewritten.contains("#[__kobo_strict]"),
            "rewritten source should contain marker attribute"
        );
        // The rewritten source should be parseable by syn
        assert!(
            syn::parse_file(&rewritten).is_ok(),
            "rewritten source should parse"
        );
    }

    // Test 2: @strict fn name() → is_strict=true, is_async=false
    #[test]
    fn test_strict_fn_preprocessed() {
        let source = "@strict fn compute() -> u64 { 42 }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(!markers.is_empty(), "should find @strict marker on fn");
        assert!(
            syn::parse_file(&rewritten).is_ok(),
            "should parse as valid Rust"
        );
    }

    // Test 3: @strict async fn → is_strict=true, is_async=true
    #[test]
    fn test_strict_async_fn_preprocessed() {
        let source = "@strict async fn handler() {}";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(
            !markers.is_empty(),
            "should find @strict marker on async fn"
        );
        assert!(
            syn::parse_file(&rewritten).is_ok(),
            "should parse as valid Rust"
        );
    }

    // Test 4: fn without @strict → no markers (no false positives)
    #[test]
    fn test_no_strict_no_markers() {
        let source = "fn normal() { let x = 1; }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(
            markers.is_empty(),
            "no markers expected in non-@strict source"
        );
        assert_eq!(source, rewritten, "source should be unchanged");
    }

    // Test 5: block without @strict → no markers
    #[test]
    fn test_plain_block_no_markers() {
        let source = "fn f() { { let x = 1; } }";
        let configs = strict_keyword_configs();
        let (_rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(markers.is_empty(), "no markers expected for plain block");
    }

    // Test 6: @strict block preserves keyword byte offset as marker
    #[test]
    fn test_strict_keyword_span_recorded() {
        let source = "fn f() { @strict { 1 + 1 } }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(!markers.is_empty());
        let marker = &markers[0];
        // The original source should have @strict starting at byte 9
        let strict_start = source.find("@strict").unwrap();
        assert_eq!(
            marker.original_span.0, strict_start,
            "marker should record original @strict byte offset"
        );
        assert!(
            rewritten.contains("#[__kobo_strict]"),
            "marker attribute must appear in rewritten source"
        );
    }

    // Test 7: @strict inside closure body → error (Trap 19)
    #[test]
    fn test_strict_inside_closure_rejected() {
        let source = "fn f() { let x = || { @strict { 1 } }; }";
        let configs = strict_keyword_configs();
        let result = crate::preprocess::preprocess_strict_reject_invalid(source, &configs);
        assert!(
            result.is_err(),
            "@strict inside closure body should be rejected"
        );
    }

    // Test 8: @strict as sub-expression → error (Trap 20)
    #[test]
    fn test_strict_as_subexpression_rejected() {
        let source = "fn f() { let x = @strict { 1 }; }";
        let configs = strict_keyword_configs();
        let result = crate::preprocess::preprocess_strict_reject_invalid(source, &configs);
        assert!(
            result.is_err(),
            "@strict as sub-expression should be rejected"
        );
    }

    // Test 9: @strict inside macro_rules! body → preprocessor skips it (Trap 21)
    #[test]
    fn test_strict_inside_macro_rules_skipped() {
        let source = r#"macro_rules! m { () => { @strict { 1 } } }"#;
        let configs = strict_keyword_configs();
        // The preprocessor should NOT rewrite @strict inside macro_rules! body.
        // The source should remain unchanged (no markers produced).
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(
            markers.is_empty(),
            "@strict inside macro_rules! should not produce markers"
        );
        assert_eq!(source, rewritten, "macro_rules! body should be untouched");
    }

    // Test 10: multiple @strict blocks in one function → both detected
    #[test]
    fn test_multiple_strict_blocks_detected() {
        let source = "fn f() { @strict { 1 } @strict { 2 } }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert_eq!(markers.len(), 2, "both @strict blocks should be detected");
        assert_eq!(
            rewritten.matches("#[__kobo_strict]").count(),
            2,
            "two marker attributes expected"
        );
    }

    // Test 11: @strict fn with parameters → is_strict=true, params preserved
    #[test]
    fn test_strict_fn_with_params_preserved() {
        let source = "@strict fn compute(x: i32, y: &[u8]) -> u64 { 0 }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        assert!(!markers.is_empty(), "marker expected on strict fn");
        // Parameters must be preserved verbatim
        assert!(
            rewritten.contains("x: i32"),
            "parameter x: i32 must be preserved"
        );
        assert!(
            rewritten.contains("y: &[u8]"),
            "parameter y: &[u8] must be preserved"
        );
        assert!(
            syn::parse_file(&rewritten).is_ok(),
            "should parse as valid Rust"
        );
    }

    // Test 12: #[__kobo_strict] does NOT appear in parse output AST identity
    // (the attribute is stripped by postprocess; Invariant C06)
    #[test]
    fn test_marker_attribute_stripped_after_parse() {
        use crate::preprocess::postprocess_strict_markers;
        let source = "fn f() { @strict { let x = 1; } }";
        let configs = strict_keyword_configs();
        let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);
        let mut file = syn::parse_file(&rewritten).expect("must parse");
        postprocess_strict_markers(&mut file, &markers).expect("postprocess must succeed");
        // Verify no #[__kobo_strict] remains in the AST
        // Walk items checking no attribute named __kobo_strict remains
        for item in &file.items {
            if let syn::Item::Fn(func) = item {
                for stmt in &func.block.stmts {
                    if let syn::Stmt::Expr(syn::Expr::Block(block), _) = stmt {
                        for attr in &block.attrs {
                            let path_str = attr
                                .path()
                                .get_ident()
                                .map(|i| i.to_string())
                                .unwrap_or_default();
                            assert_ne!(
                                path_str, "__kobo_strict",
                                "marker attribute must be stripped from final AST"
                            );
                        }
                    }
                }
            }
        }
    }
}
