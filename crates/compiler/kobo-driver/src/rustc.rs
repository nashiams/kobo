mod json;
mod remap;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use kobo_codegen::KoboSourceMap;
use kobo_ir::FileId;

use crate::session::CompileSession;

use self::remap::{remap_rustc_output, unparsed_output_diagnostic};

pub fn binary_path_for(rs_path: &Path) -> PathBuf {
    rs_path.with_extension(std::env::consts::EXE_EXTENSION)
}

pub fn compile_and_remap(
    session: &mut CompileSession,
    rs_path: &Path,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> Result<PathBuf, ()> {
    let binary_path = binary_path_for(rs_path);
    let output = run_rustc(rs_path, &binary_path, source_map, kobo_file_id, session)?;
    if output.status.success() {
        return Ok(binary_path);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = remap_rustc_output(&stderr, source_map, kobo_file_id);
    if diagnostics.is_empty() {
        session.push_diagnostic(unparsed_output_diagnostic(
            stderr.trim(),
            source_map,
            kobo_file_id,
        ));
    } else {
        session.diagnostics.extend(diagnostics);
    }

    Err(())
}

fn run_rustc(
    rs_path: &Path,
    binary_path: &Path,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
    session: &mut CompileSession,
) -> Result<Output, ()> {
    Command::new("rustc")
        .arg("--edition=2021")
        .arg("--error-format=json")
        .arg(rs_path)
        .arg("-o")
        .arg(binary_path)
        .output()
        .map_err(|error| {
            session.push_diagnostic(unparsed_output_diagnostic(
                &format!("failed to invoke rustc: {error}"),
                source_map,
                kobo_file_id,
            ));
        })
}
