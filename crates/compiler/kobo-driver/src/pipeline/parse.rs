use std::path::Path;

use kobo_errors::{DiagLabel, DiagnosticRelatedInfo, KDiagnostic, TextEdit};
use kobo_ir::Kir;
use kobo_ir::KoboSpan;
use kobo_parser::{
    collect_strict_items_from_syn, mode_parse::parse_file_mode, parse_file_recovering,
    postprocess_strict_markers, preprocess_bridge_blocks_mapped, preprocess_kobo_keywords_mapped,
    preprocess_spawn_blocks_mapped, preprocess_strict_reject_invalid, v05_keyword_configs,
    KoboFile, PreprocessSourceMap, RecoveryMode,
};
use kobo_transform::{build_kir, TransformOptions};

use crate::filesystem::read_kobo_file;
use crate::session::CompileSession;

pub fn run_kir_phase(session: &mut CompileSession, input: &Path) -> Result<(KoboFile, Kir), ()> {
    let source = match read_kobo_file(input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("kobo: io error: {error}");
            return Err(());
        }
    };
    let file_id = session.register_source_file(input.to_path_buf(), source.clone());

    // S-26: Per-module mode — file attribute overrides Kobo.toml, CLI overrides both.
    if !session.cli_mode_override {
        match parse_file_mode(&source) {
            Ok(Some(file_mode)) => {
                session.config.mode = file_mode;
            }
            Ok(None) => {} // no file-level attribute, keep current mode
            Err(e) => {
                eprintln!("kobo: {e}");
                return Err(());
            }
        }
    }

    // v0.5 preprocessing: rewrite @strict → marker attributes before syn parse.
    let configs = v05_keyword_configs();
    if let Err(e) = preprocess_strict_reject_invalid(&source, &configs) {
        eprintln!("kobo: preprocess error: {e}");
        return Err(());
    }
    let strict_mapped = preprocess_kobo_keywords_mapped(&source, file_id, &configs);
    let mut rewritten = strict_mapped.rewritten;
    let markers = strict_mapped.metadata;
    let mut preprocess_source_map = strict_mapped.source_map;

    // v0.8: rewrite spawn { ... } → __kobo_spawn_block!({ ... }) before syn parse.
    let spawn_mapped = preprocess_spawn_blocks_mapped(&rewritten, file_id);
    rewritten = spawn_mapped.rewritten;
    preprocess_source_map = spawn_mapped.source_map.compose_with(&preprocess_source_map);

    // S-57: rewrite sync { } / async { } bridge blocks before syn parse.
    let bridge_mapped = preprocess_bridge_blocks_mapped(&rewritten, file_id);
    rewritten = bridge_mapped.rewritten;
    preprocess_source_map = bridge_mapped
        .source_map
        .compose_with(&preprocess_source_map);

    let recovery_mode = if session.config.enable_parse_recovery {
        RecoveryMode::Recover
    } else {
        RecoveryMode::FailFast
    };
    let outcome = parse_file_recovering(&rewritten, file_id, &mut session.id_gen, recovery_mode);
    let parse_diagnostics = outcome
        .diagnostics
        .into_iter()
        .map(|diagnostic| remap_diagnostic_to_original_source(diagnostic, &preprocess_source_map))
        .collect::<Vec<_>>();
    let poisoned_spans = outcome
        .poisoned_spans
        .into_iter()
        .map(|span| remap_span_to_original(span, &preprocess_source_map))
        .collect::<Vec<_>>();
    session.diagnostics.extend(parse_diagnostics);
    session.poisoned_spans.extend(poisoned_spans);

    let Some(mut kobo_file) = outcome.file else {
        return Err(());
    };

    // Validate #[kobo::handler] usage (must be async fn).
    if let Err(e) = kobo_parser::validate_handler_attributes(kobo_file.syn_file()) {
        eprintln!("kobo: {e}");
        return Err(());
    }

    // S-3: Collect #[kobo::engine] structs for downstream constraint ceilings.
    let engine_structs = kobo_parser::collect_engine_structs(kobo_file.syn_file());
    if !engine_structs.is_empty() {
        session.engine_struct_names = engine_structs
            .iter()
            .map(|e| e.struct_name.clone())
            .collect();
    }

    // Collect @strict blocks/fns before stripping marker attributes.
    let (strict_blocks, strict_fns) =
        collect_strict_items_from_syn(kobo_file.syn_file(), &rewritten, file_id);
    if let Err(e) = postprocess_strict_markers(kobo_file.syn_file_mut(), &markers) {
        eprintln!("kobo: postprocess error: {e}");
        return Err(());
    }
    kobo_file.set_strict_items(strict_blocks, strict_fns);

    let mut kir = build_kir(
        &kobo_file,
        &mut session.id_gen,
        TransformOptions {
            small_struct_clone_threshold_bytes: session.config.small_struct_clone_threshold_bytes,
            copy_types: session.config.copy_types.clone(),
            mutating_methods: session.config.mutating_methods.clone(),
        },
    );

    // G5: copy relaxed fn ranges into session so the rendering path can filter warnings.
    session.relaxed_fn_ranges = kir.relaxed_fn_ranges().to_vec();

    // S-3: Mark KIR nodes whose binding type matches an engine struct.
    if !session.engine_struct_names.is_empty() {
        let mut ceiling_nodes = std::collections::HashSet::new();
        let engine_ast_ids = kobo_file.engine_typed_binding_ids(&session.engine_struct_names);
        for ast_id in engine_ast_ids {
            if let Some(kir_id) = kir.kir_for_ast(ast_id) {
                ceiling_nodes.insert(kir_id);
            }
        }
        kir.set_engine_ceiling_nodes(ceiling_nodes);
    }

    Ok((kobo_file, kir))
}

fn remap_diagnostic_to_original_source(
    mut diagnostic: KDiagnostic,
    source_map: &PreprocessSourceMap,
) -> KDiagnostic {
    diagnostic.primary = remap_label(diagnostic.primary, source_map);
    diagnostic.secondary = diagnostic
        .secondary
        .into_iter()
        .map(|label| remap_label(label, source_map))
        .collect();
    diagnostic.related = diagnostic
        .related
        .into_iter()
        .map(|related| DiagnosticRelatedInfo {
            span: remap_span_to_original(related.span, source_map),
            message: related.message,
        })
        .collect();
    diagnostic.suggestions = diagnostic
        .suggestions
        .into_iter()
        .map(|mut suggestion| {
            suggestion.edits = suggestion
                .edits
                .into_iter()
                .map(|edit| TextEdit {
                    span: remap_span_to_original(edit.span, source_map),
                    replacement: edit.replacement,
                })
                .collect();
            suggestion
        })
        .collect();
    diagnostic.suppressed_by = diagnostic
        .suppressed_by
        .map(|span| remap_span_to_original(span, source_map));
    diagnostic
}

fn remap_label(mut label: DiagLabel, source_map: &PreprocessSourceMap) -> DiagLabel {
    label.span = remap_span_to_original(label.span, source_map);
    label
}

fn remap_span_to_original(span: KoboSpan, source_map: &PreprocessSourceMap) -> KoboSpan {
    source_map.rewritten_span_to_original(span).unwrap_or(span)
}
