use std::path::Path;

use kobo_ir::Kir;
use kobo_parser::{
    collect_strict_items_from_syn, mode_parse::parse_file_mode, parse_file,
    postprocess_strict_markers, preprocess_kobo_keywords, preprocess_spawn_blocks,
    preprocess_strict_reject_invalid, v05_keyword_configs, KoboFile,
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
    let (rewritten, markers) = preprocess_kobo_keywords(&source, &configs);

    // v0.8: rewrite spawn { ... } → __kobo_spawn_block!({ ... }) before syn parse.
    let (rewritten, _spawn_infos) = preprocess_spawn_blocks(&rewritten, file_id);

    // S-57: rewrite sync { } / async { } bridge blocks before syn parse.
    let (rewritten, _bridge_infos) = kobo_parser::preprocess_bridge_blocks(&rewritten);

    let mut kobo_file = match parse_file(&rewritten, file_id, &mut session.id_gen) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("kobo: parse error: {error}");
            return Err(());
        }
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
