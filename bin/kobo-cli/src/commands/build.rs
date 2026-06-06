use anyhow::Context;
use kobo_driver::{load_config, run_codegen_pipeline, DriverError};
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
        let selection = if guarantee_profile.is_some() {
            policy::ProfileSelection::ExplicitCli
        } else {
            policy::ProfileSelection::ConfigDefault
        };
        let loaded = policy::load_effective_policy_with_selection(None, profile, selection)?;
        policy::print_policy_json(&loaded)?;
        return Ok(());
    }

    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let mut config = load_config(&cwd)?;

    if let Some(policy) = cli_policy {
        config.guarantee_policy = policy;
    }

    let output = match kobo_driver::run_build_pipeline(&config, &cwd) {
        Ok(output) => output,
        Err(DriverError::CargoBuildFailed { stderr, exit_code }) => {
            emit_cargo_compatibility_diagnostic(&stderr, exit_code, error_format)?;
            anyhow::bail!("cargo build failed (exit code {exit_code}): {stderr}");
        }
        Err(error) => return Err(error.into()),
    };

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

fn emit_cargo_compatibility_diagnostic(
    stderr: &str,
    exit_code: i32,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let message = "Cargo build failed while preserving normal Cargo dependency behavior";
    match error_format {
        ErrorFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "code": "K0128",
                "message": message,
                "exit_code": exit_code,
                "cargo_stderr": stderr,
            }))?
        ),
        ErrorFormat::Human => {
            eprintln!("error[K0128]: {message}");
            eprint!("{stderr}");
        }
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
        let selection = if guarantee_profile.is_some() {
            policy::ProfileSelection::ExplicitCli
        } else {
            policy::ProfileSelection::ConfigDefault
        };
        Some(policy::load_effective_policy_with_selection(
            Some(file),
            profile,
            selection,
        )?)
    } else {
        let base_policy = cli_policy.clone().unwrap_or_default();
        policy::load_configured_release_policy(Some(file), &base_policy)?
    };
    if let Some(loaded) = guarantee_policy.as_ref() {
        if let Some(downgrade) = loaded.downgrade() {
            policy::emit_downgrade(downgrade, error_format)?;
            anyhow::bail!("guarantee policy downgrade requires reason ledger entry");
        }
        if print_policy.is_some() {
            policy::print_policy_json(loaded)?;
            return Ok(());
        }
    }

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
    enforce_configured_new_debt_gate(&session, guarantee_policy.as_ref(), error_format)?;
    if has_error || strict_ownership_line.is_some() {
        if let Some(line) = strict_ownership_line {
            anyhow::bail!(
                "release ownership guarantee failed with K0032 ownership debt at line {line}"
            );
        }
        return Err(super::diagnostics_emitted());
    }
    if session.guarantee_policy().is_release() {
        super::watch::enforce_release_lifecycle_gate(file, error_format)?;
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

fn enforce_configured_new_debt_gate(
    session: &kobo_driver::CompileSession,
    effective_policy: Option<&policy::EffectiveGuaranteePolicy>,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    if !effective_policy.is_some_and(policy::EffectiveGuaranteePolicy::denies_new_debt) {
        return Ok(());
    }
    let Some(diagnostic) = session
        .visible_diagnostics()
        .find(|diagnostic| diagnostic.severity != Severity::Note)
    else {
        return Ok(());
    };
    emit_new_debt_gate_failure(diagnostic.code, error_format)?;
    Err(super::diagnostics_emitted())
}

fn emit_new_debt_gate_failure(code: KErrorCode, error_format: ErrorFormat) -> anyhow::Result<()> {
    let message = format!(
        "ci.release deny_new_debt blocked new guarantee debt reported by {}",
        code.as_str()
    );
    match error_format {
        ErrorFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "kind": "ci_release_gate",
                "gate": "deny_new_debt",
                "code": code.as_str(),
                "message": message,
            }))?
        ),
        ErrorFormat::Human => eprintln!("error: {message}"),
    }
    Ok(())
}
