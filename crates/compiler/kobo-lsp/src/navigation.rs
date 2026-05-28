use std::path::Path;

use kobo_codegen::{CodegenOptions, KoboSourceMap};
use kobo_ir::{Kir, KoboSpan, ScenarioOpKind, ScenarioProgram, SolutionMap};
use kobo_parser::PreprocessSourceMap;
use serde_json::{json, Value};

use crate::protocol::{
    protocol_range_json, ProtocolNavigationSite, ProtocolRange, ProtocolRustNavigation,
};
use crate::ranges::{original_span_for, protocol_range_from_rs_span, protocol_range_from_span};

pub(crate) fn compiler_rust_navigation(
    uri: &str,
    source: &str,
    preprocess_source_map: &PreprocessSourceMap,
    ast: &kobo_parser::KoboFile,
    kir: &Kir,
    programs: &[ScenarioProgram],
) -> Option<ProtocolRustNavigation> {
    let navigation_site = first_navigation_site(source, programs, preprocess_source_map)?;
    let codegen_output = kobo_codegen::codegen_file(
        kir,
        ast,
        &SolutionMap::new(),
        Path::new("main.kobo"),
        Path::new("main.rs"),
        &CodegenOptions::default(),
    );
    let generated_range =
        generated_range_from_source_map(&codegen_output.source_map, navigation_site.span)
            .or_else(|| {
                generated_binding_range(&codegen_output.rs_source, &navigation_site.binding)
            })
            .unwrap_or_else(|| navigation_site.source_range.clone());

    Some(ProtocolRustNavigation {
        generated_uri: generated_uri_for(uri),
        source_map_uri: source_map_uri_for(uri),
        source_range: navigation_site.source_range,
        generated_range,
        mapping_count: codegen_output.source_map.x_kobo_mappings.len(),
    })
}

fn first_navigation_site(
    source: &str,
    programs: &[ScenarioProgram],
    preprocess_source_map: &PreprocessSourceMap,
) -> Option<ProtocolNavigationSite> {
    programs.iter().find_map(|program| {
        program
            .operations
            .iter()
            .find_map(|operation| match &operation.kind {
                ScenarioOpKind::CreateObligation { binding, .. } => {
                    let original_span = original_span_for(preprocess_source_map, operation.span);
                    Some(ProtocolNavigationSite {
                        binding: binding.clone(),
                        span: operation.span,
                        source_range: protocol_range_from_span(source, original_span),
                    })
                }
                _ => None,
            })
    })
}

fn generated_range_from_source_map(
    source_map: &KoboSourceMap,
    source_span: KoboSpan,
) -> Option<ProtocolRange> {
    source_map
        .lookup_rs_spans(source_span)
        .into_iter()
        .next()
        .map(protocol_range_from_rs_span)
}

fn generated_binding_range(generated_source: &str, binding: &str) -> Option<ProtocolRange> {
    generated_source
        .lines()
        .enumerate()
        .find_map(|(line, text)| {
            let character_start = text.find(binding)?;
            Some(ProtocolRange {
                line,
                character_start,
                character_end: character_start + binding.len(),
            })
        })
}

pub(crate) fn source_map_metadata(navigation: &ProtocolRustNavigation) -> Value {
    json!({
        "delegate": "rust-analyzer",
        "source_map": navigation.source_map_uri,
        "generated_uri": navigation.generated_uri,
        "source_range": protocol_range_json(&navigation.source_range),
        "generated_range": protocol_range_json(&navigation.generated_range),
        "mapping_count": navigation.mapping_count,
    })
}

pub(crate) fn generated_uri_for(uri: &str) -> String {
    uri.strip_suffix(".kobo")
        .map(|stem| format!("{stem}.rs"))
        .unwrap_or_else(|| format!("{uri}.rs"))
}

pub(crate) fn source_map_uri_for(uri: &str) -> String {
    uri.strip_suffix(".kobo")
        .map(|stem| format!("{stem}.kobo.map"))
        .unwrap_or_else(|| format!("{uri}.kobo.map"))
}
