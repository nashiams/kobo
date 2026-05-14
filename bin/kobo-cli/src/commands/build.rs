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
    let mut output = String::new();
    for (line_index, line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        if let Some(site) = error_sites.iter().find(|site| site.line == line_number) {
            if let Some(question) = line.rfind('?') {
                if !line[..question].contains("map_err(") {
                    output.push_str(&line[..question]);
                    output.push_str(&format!(".map_err(KoboTypedError::{})", site.variant));
                    output.push_str(&line[question..]);
                    output.push('\n');
                    continue;
                }
            }
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn error_sites(source: &str) -> Vec<ErrorSite> {
    source
        .lines()
        .enumerate()
        .filter_map(|(line_index, line)| {
            if !line.contains('?') {
                return None;
            }
            let (operation, variant, source_error) = classify_question_operation(line);
            Some(ErrorSite {
                line: line_index + 1,
                operation,
                variant,
                source_error,
            })
        })
        .collect()
}

fn classify_question_operation(line: &str) -> (&'static str, &'static str, &'static str) {
    if line.contains("read_to_string") {
        ("read_to_string", "ReadToString", "std::io::Error")
    } else if line.contains("write(") || line.contains("write_all(") {
        ("write", "Write", "std::io::Error")
    } else if line.contains("File::open") || line.contains("OpenOptions::") {
        ("open", "Open", "std::io::Error")
    } else {
        ("io", "Io", "std::io::Error")
    }
}
