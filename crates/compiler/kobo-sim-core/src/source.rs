use std::path::Path;

use kobo_codegen::{codegen_file, CodegenOptions};
use kobo_ir::{FileId, NodeIdGen, ScenarioProgram, SolutionMap};
use kobo_parser::{
    collect_strict_items_from_syn, parse_file, postprocess_strict_markers,
    preprocess_bridge_blocks, preprocess_concurrent_sugar, preprocess_kobo_keywords,
    preprocess_spawn_blocks, preprocess_strict_reject_invalid, strict_keyword_configs,
    KeywordMarker,
};
use kobo_transform::{build_kir, TransformOptions};

use crate::digest;
use crate::error::{Result, SimCoreError};

pub(crate) struct SourceReplayArtifacts {
    pub program: ScenarioProgram,
    pub generated_rust: String,
}

pub(crate) fn compile_source_for_replay(
    source: &str,
    target: &str,
    _profile: &str,
) -> Result<SourceReplayArtifacts> {
    let file_id = FileId(0);
    let mut id_gen = NodeIdGen::new();
    let PreprocessedReplaySource { rewritten, markers } = preprocess_source(source, file_id)?;
    let mut kobo_file = parse_file(&rewritten, file_id, &mut id_gen)
        .map_err(|source| SimCoreError::source_compile("parse", source.to_string()))?;

    kobo_parser::validate_handler_attributes(kobo_file.syn_file())
        .map_err(|source| SimCoreError::source_compile("handler validation", source.to_string()))?;

    let (strict_blocks, strict_fns) =
        collect_strict_items_from_syn(kobo_file.syn_file(), &rewritten, file_id);
    postprocess_strict_markers(kobo_file.syn_file_mut(), &markers).map_err(|source| {
        SimCoreError::source_compile("strict marker cleanup", source.to_string())
    })?;
    kobo_file.set_strict_items(strict_blocks, strict_fns);

    let kir = build_kir(&kobo_file, &mut id_gen, TransformOptions::default());
    let output = codegen_file(
        &kir,
        &kobo_file,
        &SolutionMap::new(),
        Path::new("source.kobo"),
        Path::new("source.rs"),
        &CodegenOptions::default(),
    );
    let mut program = kir
        .scenario_programs()
        .iter()
        .find(|program| program.target == target)
        .cloned()
        .ok_or_else(|| SimCoreError::SourceScenarioMissing {
            target: target.to_owned(),
        })?;
    program.source_hash = digest::stable_hash(source);
    program.target = target.to_owned();

    Ok(SourceReplayArtifacts {
        program,
        generated_rust: output.rs_source,
    })
}

struct PreprocessedReplaySource {
    rewritten: String,
    markers: Vec<KeywordMarker>,
}

fn preprocess_source(source: &str, file_id: FileId) -> Result<PreprocessedReplaySource> {
    let configs = strict_keyword_configs();
    let (rewritten, _) = preprocess_concurrent_sugar(source);
    preprocess_strict_reject_invalid(&rewritten, &configs).map_err(|source| {
        SimCoreError::source_compile("strict keyword validation", source.to_string())
    })?;
    let (rewritten, markers) = preprocess_kobo_keywords(&rewritten, &configs);
    let (rewritten, _) = preprocess_spawn_blocks(&rewritten, file_id);
    let (rewritten, _) = preprocess_bridge_blocks(&rewritten);
    Ok(PreprocessedReplaySource { rewritten, markers })
}
