use anyhow::Context;
use kobo_driver::{load_config, run_codegen_pipeline};
use kobo_errors::{ColorMode, KErrorCode, Severity};
use kobo_ir::GuaranteePolicy;

use std::path::Path;

use crate::{ErrorFormat, GuaranteeProfileArg, PolicyOutputFormat};

use super::{
    policy,
    session::{build_session, render_diagnostics_with_format},
};

pub(super) fn cmd_build(
    cli_policy: Option<GuaranteePolicy>,
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
            cli_policy,
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

    if let Some(policy) = cli_policy {
        config.guarantee_policy = policy;
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
    cli_policy: Option<GuaranteePolicy>,
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

    let session_policy = guarantee_policy
        .as_ref()
        .map(|policy| policy.compiler_policy().clone())
        .or(cli_policy);
    let mut session = build_session(file, session_policy)?;
    let mut artifacts = run_codegen_pipeline(&mut session, file).map_err(|()| {
        render_diagnostics_with_format(&session, error_format, color_mode);
        super::diagnostics_emitted()
    })?;
    if let Some(policy) = guarantee_policy.as_ref() {
        artifacts = kobo_driver::apply_error_policy_sites(artifacts, policy.error_policy_name());
    }
    let _error_policy_sites = artifacts.error_policy_sites.len();
    render_diagnostics_with_format(&session, error_format, color_mode);
    let strict_ownership_line = if session.guarantee_policy().is_release() {
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
        let rs_source = artifacts.rs_source;
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
