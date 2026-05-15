use anyhow::Context;
use kobo_driver::{load_config, run_codegen_pipeline, KoboMode};
use kobo_errors::{ColorMode, KErrorCode, Severity};

use std::path::Path;

use crate::{ErrorFormat, GuaranteeProfileArg, PolicyOutputFormat};

use super::{
    policy,
    session::{build_session, render_diagnostics_with_format},
};

pub(super) fn cmd_build(
    cli_mode: Option<KoboMode>,
    guarantee_profile: Option<GuaranteeProfileArg>,
    print_policy: Option<PolicyOutputFormat>,
    error_format: ErrorFormat,
    color_mode: ColorMode,
    emit_rust: bool,
    file: Option<&Path>,
) -> anyhow::Result<()> {
    if let Some(file) = file {
        return cmd_build_file(
            file,
            cli_mode,
            guarantee_profile,
            print_policy,
            error_format,
            color_mode,
            emit_rust,
        );
    }

    if print_policy.is_some() {
        let profile = guarantee_profile.unwrap_or(GuaranteeProfileArg::Dev);
        let loaded = policy::load_effective_policy(None, profile)?;
        policy::print_policy_json(&loaded)?;
        return Ok(());
    }

    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let mut config = load_config(&cwd)?;

    if let Some(mode) = cli_mode {
        config.mode = mode;
    }

    let output = kobo_driver::run_build_pipeline(&config, &cwd)?;

    if let Some(bin) = &output.binary_path {
        eprintln!("Build succeeded: {}", bin.display());
    } else {
        eprintln!("Build succeeded (no binary produced).");
    }

    for diag in &output.diagnostics {
        eprint!("{diag}");
    }

    Ok(())
}

fn cmd_build_file(
    file: &Path,
    cli_mode: Option<KoboMode>,
    guarantee_profile: Option<GuaranteeProfileArg>,
    print_policy: Option<PolicyOutputFormat>,
    error_format: ErrorFormat,
    color_mode: ColorMode,
    emit_rust: bool,
) -> anyhow::Result<()> {
    let guarantee_policy = if guarantee_profile.is_some() || print_policy.is_some() {
        let profile = guarantee_profile.unwrap_or(GuaranteeProfileArg::Dev);
        let loaded = policy::load_effective_policy(Some(file), profile)?;
        if let Some(downgrade) = loaded.downgrade() {
            policy::emit_downgrade(downgrade, error_format)?;
            anyhow::bail!("guarantee policy downgrade requires reason ledger entry");
        }
        if print_policy.is_some() {
            policy::print_policy_json(&loaded)?;
            return Ok(());
        }
        Some(loaded)
    } else {
        None
    };

    let mut session = build_session(file, cli_mode)?;
    let artifacts = run_codegen_pipeline(&mut session, file).map_err(|()| {
        render_diagnostics_with_format(&session, error_format, color_mode);
        super::diagnostics_emitted()
    })?;
    render_diagnostics_with_format(&session, error_format, color_mode);
    let strict_ownership_line = if session.mode().is_strict() {
        session
            .visible_diagnostics()
            .find(|diagnostic| diagnostic.code == KErrorCode::K0032)
            .and_then(|diagnostic| {
                session
                    .file_set()
                    .get(diagnostic.primary.span.file_id)
                    .map(|file| file.line_col(diagnostic.primary.span.start).0)
            })
    } else {
        None
    };
    let has_error = session
        .visible_diagnostics()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    if has_error || strict_ownership_line.is_some() {
        if let Some(line) = strict_ownership_line {
            anyhow::bail!(
                "release ownership guarantee failed with K0001 ownership debt at line {line}"
            );
        }
        return Err(super::diagnostics_emitted());
    }

    if emit_rust {
        println!("generated: {}", artifacts.rs_path.display());
        let rs_source = if let Some(policy) = guarantee_policy.as_ref() {
            apply_error_policy_posture(&artifacts.rs_source, policy)
        } else {
            artifacts.rs_source
        };
        std::fs::write(&artifacts.rs_path, &rs_source)
            .with_context(|| format!("failed to write {}", artifacts.rs_path.display()))?;
        println!("{rs_source}");
    } else {
        eprintln!("Build succeeded: {}", artifacts.rs_path.display());
    }

    if let Some(policy) = guarantee_policy.as_ref() {
        policy::emit_policy_summary(policy);
    }

    Ok(())
}

fn apply_error_policy_posture(source: &str, policy: &policy::GuaranteePolicy) -> String {
    let error_sites = error_sites(source);
    match policy.error_policy_name() {
        "ergonomic" => source.replace("std::io::Error", "Box<dyn std::error::Error>"),
        "typed" => typed_error_policy_source(source, &error_sites),
        "explicit" => {
            let mut output = source.to_owned();
            if !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("// kobo: explicit error policy evidence required\n");
            for site in error_sites {
                output.push_str(&format!(
                    "// kobo: error_site line {} operation {} source {}\n",
                    site.line, site.operation, site.source_error
                ));
            }
            output
        }
        _ => source.to_owned(),
    }
}

#[derive(Clone, Debug)]
struct ErrorSite {
    offset: usize,
    line: usize,
    operation: &'static str,
    variant: &'static str,
    source_error: &'static str,
}

fn typed_error_policy_source(source: &str, error_sites: &[ErrorSite]) -> String {
    if source.contains("enum KoboTypedError") {
        return source.to_owned();
    }

    let rewritten = rewrite_question_error_sites(source, error_sites)
        .replace("std::io::Error", "KoboTypedError");
    let mut variants = Vec::new();
    variants.push(("Io", "io", "std::io::Error"));
    for site in error_sites {
        if !variants
            .iter()
            .any(|(variant, _, _)| *variant == site.variant)
        {
            variants.push((site.variant, site.operation, site.source_error));
        }
    }

    let declarations = variants
        .iter()
        .map(|(variant, _, source_error)| format!("    {variant}({source_error}),"))
        .collect::<Vec<_>>()
        .join("\n");
    let display_arms = variants
        .iter()
        .map(|(variant, operation, _)| {
            format!(
                "            Self::{variant}(error) => write!(f, \"{operation} failed: {{error}}\"),"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "#[derive(Debug)]\n#[allow(dead_code)]\nenum KoboTypedError {{\n{declarations}\n}}\n\nimpl std::fmt::Display for KoboTypedError {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match self {{\n{display_arms}\n        }}\n    }}\n}}\n\nimpl std::error::Error for KoboTypedError {{}}\n\nimpl From<std::io::Error> for KoboTypedError {{\n    fn from(error: std::io::Error) -> Self {{\n        Self::Io(error)\n    }}\n}}\n\n{rewritten}"
    )
}

fn rewrite_question_error_sites(source: &str, error_sites: &[ErrorSite]) -> String {
    if error_sites.is_empty() {
        return source.to_owned();
    }

    let mut output = String::with_capacity(source.len() + error_sites.len() * 32);
    let mut cursor = 0usize;
    for site in error_sites {
        if site.offset > source.len() || site.offset < cursor {
            continue;
        }
        if expression_already_maps_error(&source[..site.offset]) {
            continue;
        }
        output.push_str(&source[cursor..site.offset]);
        output.push_str(&format!(".map_err(KoboTypedError::{})", site.variant));
        cursor = site.offset;
    }
    output.push_str(&source[cursor..]);
    output
}

fn error_sites(source: &str) -> Vec<ErrorSite> {
    question_operator_offsets(source)
        .into_iter()
        .map(|offset| {
            let context = expression_context_before(source, offset);
            let (operation, variant, source_error) = classify_question_operation(context);
            ErrorSite {
                offset,
                line: one_based_line_for_offset(source, offset),
                operation,
                variant,
                source_error,
            }
        })
        .collect()
}

fn classify_question_operation(context: &str) -> (&'static str, &'static str, &'static str) {
    if context.contains("read_to_string") {
        ("read_to_string", "ReadToString", "std::io::Error")
    } else if context.contains("write(") || context.contains("write_all(") {
        ("write", "Write", "std::io::Error")
    } else if context.contains("File::open") || context.contains("OpenOptions::") {
        ("open", "Open", "std::io::Error")
    } else {
        ("io", "Io", "std::io::Error")
    }
}

fn question_operator_offsets(source: &str) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut offsets = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'?' => {
                offsets.push(index);
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'"' => {
                index = skip_quoted(bytes, index, b'"');
            }
            b'\'' => {
                index = skip_quoted(bytes, index, b'\'');
            }
            b'r' if raw_string_hashes(bytes, index).is_some() => {
                index = skip_raw_string(bytes, index);
            }
            _ => {
                index += 1;
            }
        }
    }
    offsets
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn raw_string_hashes(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'r') {
        return None;
    }
    let mut index = start + 1;
    let mut hashes = 0usize;
    while bytes.get(index) == Some(&b'#') {
        hashes += 1;
        index += 1;
    }
    (bytes.get(index) == Some(&b'"')).then_some(hashes)
}

fn skip_raw_string(bytes: &[u8], start: usize) -> usize {
    let Some(hashes) = raw_string_hashes(bytes, start) else {
        return start + 1;
    };
    let mut index = start + 1 + hashes + 1;
    while index < bytes.len() {
        if bytes[index] == b'"'
            && (0..hashes).all(|hash| bytes.get(index + 1 + hash) == Some(&b'#'))
        {
            return (index + 1 + hashes).min(bytes.len());
        }
        index += 1;
    }
    bytes.len()
}

fn expression_context_before(source: &str, offset: usize) -> &str {
    let prefix = &source[..offset.min(source.len())];
    let start = prefix
        .rfind(|ch| matches!(ch, ';' | '\n' | '{' | '}'))
        .map(|index| index + 1)
        .unwrap_or(0);
    prefix[start..].trim()
}

fn expression_already_maps_error(prefix: &str) -> bool {
    let context = expression_context_before(prefix, prefix.len());
    context.contains(".map_err(")
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
