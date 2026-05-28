use kobo_ir::{FileId, KoboSpan};
use kobo_parser::{
    preprocess_bridge_blocks_mapped, preprocess_kobo_keywords_mapped,
    preprocess_spawn_blocks_mapped, strict_keyword_configs,
};

#[test]
fn strict_rewrite_maps_generated_marker_back_to_original_keyword() {
    let source = "@strict fn main() { let x = 1; }\n";
    let configs = strict_keyword_configs();
    let mapped = preprocess_kobo_keywords_mapped(source, FileId(0), &configs);

    let generated_offset = mapped
        .rewritten
        .find("#[__kobo_strict]")
        .expect("strict marker should be generated");
    let original = mapped
        .source_map
        .rewritten_span_to_original(KoboSpan::new(
            generated_offset as u32,
            (generated_offset + "#[__kobo_strict]".len()) as u32,
            FileId(0),
        ))
        .expect("generated marker should map back to original source");

    assert_eq!(original.start, 0);
    assert_eq!(original.end, "@strict".len() as u32);
}

#[test]
fn spawn_rewrite_maps_macro_wrapper_back_to_original_spawn_block() {
    let source = "fn main() { spawn { do_work(); } }\n";
    let mapped = preprocess_spawn_blocks_mapped(source, FileId(0));

    let generated_offset = mapped
        .rewritten
        .find("__kobo_spawn_block!")
        .expect("spawn macro should be generated");
    let original = mapped
        .source_map
        .rewritten_span_to_original(KoboSpan::new(
            generated_offset as u32,
            (generated_offset + "__kobo_spawn_block!".len()) as u32,
            FileId(0),
        ))
        .expect("generated spawn macro should map back to original source");

    assert_eq!(
        &source[original.start as usize..original.end as usize],
        "spawn"
    );
}

#[test]
fn bridge_rewrite_keeps_original_span_for_recovered_parse_diagnostics() {
    let source = "fn main() { sync { let x = ; } }\n";
    let mapped = preprocess_bridge_blocks_mapped(source, FileId(0));
    let generated_offset = mapped
        .rewritten
        .find("let x")
        .expect("rewritten source should contain original statement");
    let original = mapped
        .source_map
        .rewritten_span_to_original(KoboSpan::new(
            generated_offset as u32,
            (generated_offset + "let x".len()) as u32,
            FileId(0),
        ))
        .expect("bridge rewrite should map statement back to original source");

    assert_eq!(
        &source[original.start as usize..original.end as usize],
        "let x"
    );
}
