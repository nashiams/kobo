mod json;
mod remap;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use kobo_codegen::KoboSourceMap;
use kobo_ir::FileId;

use crate::session::CompileSession;

use self::json::{parse_rustc_diagnostics, RustcJsonError};
use self::remap::{remap_rustc_output, remap_warning_diagnostic, unparsed_output_diagnostic};

static COMPILE_BINARY_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Return value of a successful call to `compile_and_remap`.
pub struct CompileOutput {
    /// Path to the compiled binary.
    pub output_path: PathBuf,
    /// Rustc warnings captured from stderr (non-empty only in checked mode).
    /// These are the RAW parsed warnings BEFORE noise filtering or span remapping.
    pub rustc_warnings: Vec<RustcJsonError>,
}

pub fn binary_path_for(rs_path: &Path) -> PathBuf {
    let counter = COMPILE_BINARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem = rs_path
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "out".into());
    let mut file_name = format!("{stem}.kobo-run-{}-{counter}", std::process::id());
    let extension = std::env::consts::EXE_EXTENSION;
    if !extension.is_empty() {
        file_name.push('.');
        file_name.push_str(extension);
    }
    rs_path.with_file_name(file_name)
}

pub fn compile_and_remap(
    session: &mut CompileSession,
    rs_path: &Path,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> Result<CompileOutput, ()> {
    let binary_path = binary_path_for(rs_path);
    let output = run_rustc(rs_path, &binary_path, source_map, kobo_file_id, session)?;
    if output.status.success() {
        // Capture warnings on the success path in checked mode.
        // Script mode skips warning parsing (conservative — no noise to filter).
        let rustc_warnings = if session.guarantee_policy().is_checked() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            parse_rustc_diagnostics(&stderr).warnings
        } else {
            vec![]
        };
        return Ok(CompileOutput {
            output_path: binary_path,
            rustc_warnings,
        });
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

/// Filter rustc warnings that are artifacts of Kobo's generated wrapper code.
/// Conservative: when in doubt, KEEP the warning [Invariant R09].
///
/// `kobo_regions` is a list of (start_line, end_line) pairs derived from
/// `// kobo:` comment annotations in the generated .rs source.
pub fn filter_wrapper_noise(
    diagnostics: &[RustcJsonError],
    kobo_regions: &[(u32, u32)],
) -> Vec<RustcJsonError> {
    diagnostics
        .iter()
        .filter(|diag| !is_wrapper_noise(diag, kobo_regions))
        .cloned()
        .collect()
}

fn is_wrapper_noise(diag: &RustcJsonError, kobo_regions: &[(u32, u32)]) -> bool {
    // Rule 1: unused_imports for Rc / RefCell wrappers → DROP
    if let Some(children_message) = diag.children.first().map(|c| c.message.as_str()) {
        let _ = children_message; // may use later for detailed matching
    }
    if is_lint(diag, "unused_imports")
        && (diag.message.contains("rc::Rc") || diag.message.contains("RefCell"))
    {
        return true;
    }
    // Rule 2: dead_code on __kobo_* symbols → DROP
    if is_lint(diag, "dead_code") && primary_label_contains(diag, "__kobo_") {
        return true;
    }
    // Rule 3: unused_variables for __kobo_* variables → DROP
    if is_lint(diag, "unused_variables")
        && (diag.message.contains("__kobo_") || primary_label_contains(diag, "__kobo_"))
    {
        return true;
    }
    // Rule 4: clippy::redundant_clone on Rc<_> or Arc<_> types → DROP
    if is_lint(diag, "clippy::redundant_clone")
        && (diag.message.contains("Rc<") || diag.message.contains("Arc<"))
    {
        return true;
    }
    // Rule 5: clippy::unnecessary_wraps involving Rc<RefCell<_>> → DROP
    if is_lint(diag, "clippy::unnecessary_wraps") && diag.message.contains("Rc<RefCell<") {
        return true;
    }
    // Rule 6: primary span inside a // kobo: annotated region → DROP
    if let Some(primary) = diag.spans.iter().find(|s| s.is_primary) {
        let line = primary.line_start as u32;
        if kobo_regions
            .iter()
            .any(|(start, end)| line >= *start && line <= *end)
        {
            return true;
        }
    }
    // Default: KEEP (conservative)
    false
}

fn is_lint(diag: &RustcJsonError, lint_name: &str) -> bool {
    if let Some(code) = &diag.code {
        return code.code == lint_name;
    }
    // Fallback: check children for the lint identifier
    diag.children.iter().any(|c| c.message.contains(lint_name))
}

fn primary_label_contains(diag: &RustcJsonError, needle: &str) -> bool {
    diag.spans.iter().find(|s| s.is_primary).is_some_and(|s| {
        s.label
            .as_deref()
            .is_some_and(|label| label.contains(needle))
            || s.file_name.contains(needle)
    })
}

/// Re-map surviving rustc warnings to.kobo spans and push to session diagnostics.
/// Warnings that cannot be remapped get a fallback "(generated code)" note.
pub fn remap_warnings_to_diagnostics(
    warnings: Vec<RustcJsonError>,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
    session: &mut CompileSession,
) {
    for warning in warnings {
        session.push_diagnostic(remap_warning_diagnostic(warning, source_map, kobo_file_id));
    }
}

/// Extract // kobo: annotated line regions from generated .rs source.
/// Returns (start_line, end_line) pairs (1-based line numbers).
pub fn extract_kobo_regions(rs_source: &str) -> Vec<(u32, u32)> {
    let mut regions = Vec::new();
    let mut in_region = false;
    let mut region_start = 0u32;

    for (i, line) in rs_source.lines().enumerate() {
        let line_no = (i + 1) as u32;
        let trimmed = line.trim();
        if trimmed.starts_with("// kobo:") || trimmed == "// kobo: begin" {
            if !in_region {
                in_region = true;
                region_start = line_no;
            }
        } else if in_region {
            regions.push((region_start, line_no - 1));
            in_region = false;
        }
    }
    if in_region {
        regions.push((region_start, rs_source.lines().count() as u32));
    }
    regions
}

/// Locate the kobo-diag rlib in the same `deps/` directory as the current
/// executable. Returns None if it cannot be found or the path is ambiguous.
///
/// This is the build-layer strategy for G1 Task 1.4 [R6-04]:
/// Generated `.rs` files emit `use kobo_diag::DiagOwner;` when diag is active.
/// Bare `rustc` cannot resolve external crates without `--extern`. We find the
/// rlib from our own Cargo target directory and pass `--extern kobo_diag=<path>`.
fn find_dependency_rlib(crate_name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // exe is e.g. target/debug/kobo or target/release/kobo
    // rlib is in target/debug/deps/lib<crate_name>-<hash>.rlib
    let deps_dir = exe.parent()?.join("deps");
    if !deps_dir.is_dir() {
        return None;
    }
    let file_prefix = format!("lib{}", crate_name.replace('-', "_"));
    // Find the most recently modified libkobo_diag-*.rlib to avoid ambiguity
    // when multiple hash suffixes exist from rebuilds.
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(&deps_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&file_prefix) && name_str.ends_with(".rlib") {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if best.as_ref().is_none_or(|(_, t)| mtime > *t) {
                            best = Some((entry.path(), mtime));
                        }
                    }
                }
            }
        }
    }
    best.map(|(path, _)| path)
}

fn add_extern_rlib(cmd: &mut Command, crate_name: &str) {
    if let Some(rlib) = find_dependency_rlib(crate_name) {
        if let Some(deps_dir) = rlib.parent() {
            cmd.arg("-L").arg(deps_dir);
        }
        cmd.arg(format!("--extern={crate_name}={}", rlib.display()));
    }
}

fn generated_source_needs_crate(rs_path: &Path, crate_name: &str) -> bool {
    std::fs::read_to_string(rs_path)
        .map(|source| source.contains(&format!("{crate_name}::")))
        .unwrap_or(false)
}

fn run_rustc(
    rs_path: &Path,
    binary_path: &Path,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
    session: &mut CompileSession,
) -> Result<Output, ()> {
    let mut cmd = Command::new("rustc");
    cmd.arg("--edition=2021")
        .arg("--error-format=json")
        .arg(rs_path)
        .arg("-o")
        .arg(binary_path);

    // Build layer [G1 §1.4 / R6-04]: when diag instrumentation is active,
    // the generated .rs file contains `use kobo_diag::DiagOwner;`. The bare
    // `rustc` invocation must resolve this extern crate, or linking fails.
    // We pass --extern kobo_diag=<rlib> and -L <deps_dir> so rustc finds it.
    if session.diag_enabled {
        add_extern_rlib(&mut cmd, "kobo_diag");
    }

    if generated_source_needs_crate(rs_path, "arc_swap") {
        add_extern_rlib(&mut cmd, "arc_swap");
    }

    cmd.output().map_err(|error| {
        session.push_diagnostic(unparsed_output_diagnostic(
            &format!("failed to invoke rustc: {error}"),
            source_map,
            kobo_file_id,
        ));
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{binary_path_for, extract_kobo_regions, filter_wrapper_noise};
    use crate::rustc::json::{RustcCode, RustcJsonError, RustcSpan};

    fn make_warning(msg: &str, lint: &str, spans: Vec<RustcSpan>) -> RustcJsonError {
        RustcJsonError {
            message: msg.to_owned(),
            code: Some(RustcCode {
                code: lint.to_owned(),
            }),
            level: "warning".to_owned(),
            spans,
            children: vec![],
        }
    }

    fn make_span(line: usize, is_primary: bool, label: Option<&str>) -> RustcSpan {
        RustcSpan {
            file_name: "generated.rs".to_owned(),
            line_start: line,
            column_start: 1,
            line_end: line,
            column_end: 10,
            is_primary,
            label: label.map(|s| s.to_owned()),
        }
    }

    #[test]
    fn binary_path_for_repeated_compile_uses_distinct_output_path() {
        let first = binary_path_for(Path::new("src/main.rs"));
        let second = binary_path_for(Path::new("src/main.rs"));
        assert_ne!(
            first, second,
            "repeated rustc invocations must not contend for the same executable or PDB"
        );
    }

    #[test]
    fn test_filter_wrapper_noise_drops_unused_rc_import() {
        let diag = make_warning("unused import: `rc::Rc`", "unused_imports", vec![]);
        let result = filter_wrapper_noise(&[diag], &[]);
        assert!(result.is_empty(), "Rc import should be filtered");
    }

    #[test]
    fn test_filter_wrapper_noise_drops_unused_refcell_import() {
        let diag = make_warning("unused import: `RefCell`", "unused_imports", vec![]);
        let result = filter_wrapper_noise(&[diag], &[]);
        assert!(result.is_empty(), "RefCell import should be filtered");
    }

    #[test]
    fn test_filter_wrapper_noise_drops_kobo_dead_code() {
        let span = make_span(5, true, Some("__kobo_guard_0"));
        let diag = make_warning("function is never used", "dead_code", vec![span]);
        let result = filter_wrapper_noise(&[diag], &[]);
        assert!(result.is_empty(), "__kobo_ dead_code should be filtered");
    }

    #[test]
    fn test_filter_wrapper_noise_drops_kobo_unused_variable() {
        let diag = make_warning(
            "unused variable: `__kobo_guard_0`",
            "unused_variables",
            vec![],
        );
        let result = filter_wrapper_noise(&[diag], &[]);
        assert!(
            result.is_empty(),
            "__kobo_ unused_variables should be filtered"
        );
    }

    #[test]
    fn test_filter_wrapper_noise_keeps_user_variable() {
        let diag = make_warning("unused variable: `data`", "unused_variables", vec![]);
        let result = filter_wrapper_noise(&[diag], &[]);
        assert_eq!(result.len(), 1, "User variable warning should be kept");
    }

    #[test]
    fn test_filter_wrapper_noise_drops_rc_redundant_clone() {
        let diag = make_warning(
            "unnecessary clone of Rc<RefCell<Vec<i32>>> value",
            "clippy::redundant_clone",
            vec![],
        );
        let result = filter_wrapper_noise(&[diag], &[]);
        assert!(result.is_empty(), "Rc<> redundant_clone should be filtered");
    }

    #[test]
    fn test_filter_wrapper_noise_drops_span_in_kobo_region() {
        let span = make_span(10, true, None);
        let diag = make_warning("some warning", "some_lint", vec![span]);
        let kobo_regions = vec![(8u32, 15u32)]; // span line 10 is inside
        let result = filter_wrapper_noise(&[diag], &kobo_regions);
        assert!(
            result.is_empty(),
            "Span inside kobo: region should be filtered"
        );
    }

    #[test]
    fn test_filter_wrapper_noise_keeps_span_outside_kobo_region() {
        let span = make_span(20, true, None);
        let diag = make_warning("user warning", "some_lint", vec![span]);
        let kobo_regions = vec![(8u32, 15u32)]; // span line 20 is outside
        let result = filter_wrapper_noise(&[diag], &kobo_regions);
        assert_eq!(result.len(), 1, "Span outside kobo: region should be kept");
    }

    #[test]
    fn test_extract_kobo_regions_basic() {
        let source = "line 1\n// kobo: wrapper\nline 3\nline 4\n";
        let regions = extract_kobo_regions(source);
        assert_eq!(regions, vec![(2, 2)]);
    }

    #[test]
    fn test_extract_kobo_regions_empty() {
        let regions = extract_kobo_regions("fn main() {}\n");
        assert!(regions.is_empty());
    }
}
