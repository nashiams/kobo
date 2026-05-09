use std::path::{Path, PathBuf};

use crate::filesystem::write_rs_file;
use crate::rustc::{
    compile_and_remap, extract_kobo_regions, filter_wrapper_noise, remap_warnings_to_diagnostics,
};
use crate::session::CompileSession;

use super::codegen::{run_codegen_pipeline, CodegenArtifacts};
use super::util::apply_lifetime_erasure;

pub fn run_and_compile(session: &mut CompileSession, input: &Path) -> Result<PathBuf, ()> {
    let artifacts = run_codegen_pipeline(session, input)?;
    compile_codegen_artifacts(session, &artifacts)
}

pub fn run_and_compile_with_lifetime_erasure(
    session: &mut CompileSession,
    input: &Path,
) -> Result<PathBuf, ()> {
    let mut artifacts = run_codegen_pipeline(session, input)?;
    let erased_source = apply_lifetime_erasure(&artifacts.rs_source, session.mode());
    if erased_source != artifacts.rs_source {
        if let Err(error) = write_rs_file(&artifacts.rs_path, &erased_source) {
            eprintln!("kobo: write error: {error}");
            return Err(());
        }
        artifacts.rs_source = erased_source;
    }

    compile_codegen_artifacts(session, &artifacts)
}

fn compile_codegen_artifacts(
    session: &mut CompileSession,
    artifacts: &CodegenArtifacts,
) -> Result<PathBuf, ()> {
    let compile_output = compile_and_remap(
        session,
        &artifacts.rs_path,
        &artifacts.source_map,
        artifacts.file_id,
    )?;

    // v0.6 G4 §4.4: In checked mode, filter wrapper noise and remap surviving
    // warnings to .kobo spans, then push into session.diagnostics [R6-02].
    // ORDER IS CRITICAL: filter(§4.2) must use .rs spans → remap(§4.3) → merge.
    if session.mode().is_checked() && !compile_output.rustc_warnings.is_empty() {
        let kobo_regions = extract_kobo_regions(&artifacts.rs_source);
        let surviving = filter_wrapper_noise(&compile_output.rustc_warnings, &kobo_regions);
        remap_warnings_to_diagnostics(surviving, &artifacts.source_map, artifacts.file_id, session);
    }

    Ok(compile_output.output_path)
}
