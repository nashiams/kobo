mod backend_debt;
mod bench;
mod bindgen;
mod boundary_projection;
mod build;
mod check;
mod debt;
mod declarations;
mod doctor;
mod ecosystem;
mod explain;
mod fix;
mod fmt;
mod formal_core;
mod init;
mod lsp_diagnostics;
mod migrate;
mod ownership_analysis;
mod perf;
mod policy;
mod proof;
mod replay;
mod run;
mod session;
mod sim;
mod sim_model;
mod summary_validation;
mod test_cmd;
mod watch;
mod witness_evidence;

use std::fmt as std_fmt;

use kobo_ir::GuaranteePolicy;

use crate::{
    resolve_guarantee_profile, GuaranteeProfileArg, KoboCommand, ProofCommand, SimCommand,
};

#[derive(Debug)]
pub(crate) struct DiagnosticExit;

impl std_fmt::Display for DiagnosticExit {
    fn fmt(&self, formatter: &mut std_fmt::Formatter<'_>) -> std_fmt::Result {
        formatter.write_str("diagnostics emitted")
    }
}

impl std::error::Error for DiagnosticExit {}

pub(crate) fn diagnostics_emitted() -> anyhow::Error {
    DiagnosticExit.into()
}

pub(crate) fn is_diagnostic_exit(error: &anyhow::Error) -> bool {
    error.downcast_ref::<DiagnosticExit>().is_some()
}

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check {
            file,
            checked,
            strict,
            profile,
            print_policy,
            pipeline,
            error_format,
            color,
            recover_parse,
            replay_critical,
            max_diagnostics,
            visible_region,
            include_budgeted,
            emit_proof,
        } => {
            let guarantee_profile = resolve_guarantee_profile(checked, strict, profile);
            check::cmd_check(
                &file,
                cli_policy(guarantee_profile),
                guarantee_profile,
                print_policy,
                pipeline,
                error_format,
                color.color_mode(),
                recover_parse,
                replay_critical,
                max_diagnostics,
                visible_region.as_deref(),
                include_budgeted,
                emit_proof,
            )
        }
        KoboCommand::Proof { command } => match command {
            ProofCommand::Emit {
                file,
                target,
                output,
                replay_grade,
            } => proof::cmd_emit(&file, target.as_deref(), output.as_deref(), replay_grade),
            ProofCommand::Verify { artifact, json } => proof::cmd_verify(&artifact, json),
        },
        KoboCommand::LspDiagnostics {
            file,
            format,
            no_project_ok,
            include_actions,
        } => lsp_diagnostics::cmd_lsp_diagnostics(&file, format, no_project_ok, include_actions),
        KoboCommand::Fmt { file } => fmt::cmd_fmt(&file),
        KoboCommand::Run {
            file,
            checked,
            strict,
            profile,
            erase_lifetimes,
        } => {
            let guarantee_profile = resolve_guarantee_profile(checked, strict, profile);
            run::cmd_run(
                &file,
                cli_policy(guarantee_profile),
                guarantee_profile,
                erase_lifetimes,
            )
        }
        KoboCommand::Inspect {
            file,
            checked,
            strict,
            clean,
            erase_lifetimes,
            scenario_metadata,
            sim,
            harness,
            cargo,
            profile,
            backend,
            trait_default,
            audit,
        } => run::cmd_inspect(
            &file,
            cli_policy(resolve_guarantee_profile(checked, strict, None)),
            clean,
            erase_lifetimes,
            scenario_metadata,
            sim,
            harness,
            cargo.as_deref(),
            profile.as_deref(),
            backend.as_deref(),
            trait_default.as_deref(),
            audit.as_deref(),
        ),
        KoboCommand::Doctor {
            deps,
            self_host,
            json,
        } => {
            let output_format = if json {
                doctor::DoctorOutputFormat::Json
            } else {
                doctor::DoctorOutputFormat::Human
            };
            let dependency_inspection = if deps {
                doctor::DependencyInspection::Requested
            } else {
                doctor::DependencyInspection::Default
            };
            let mode = if self_host {
                doctor::DoctorMode::SelfHost
            } else {
                doctor::DoctorMode::Dependencies(dependency_inspection)
            };
            doctor::cmd_doctor(doctor::DoctorOptions {
                mode,
                output_format,
            })
        }
        KoboCommand::Bindgen {
            crate_option,
            features,
            path,
        } => bindgen::cmd_bindgen(bindgen::BindgenOptions {
            path: path.as_deref(),
            crate_name: crate_option.as_deref(),
            features: features.as_deref(),
        }),
        KoboCommand::Add {
            crate_name,
            features,
            version,
            path,
            git,
            no_default_features,
            dev,
            build,
            target,
            manifest_path,
            member,
        } => ecosystem::cmd_add(
            &crate_name,
            features.as_deref(),
            version.as_deref(),
            path.as_deref(),
            git.as_deref(),
            no_default_features,
            dev,
            build,
            target.as_deref(),
            manifest_path.as_deref(),
            member.as_deref(),
        ),
        KoboCommand::AddTypes { crate_name } => ecosystem::cmd_add_types(&crate_name),
        KoboCommand::AddAdapter { crate_name } => ecosystem::cmd_add_adapter(&crate_name),
        KoboCommand::MigrateCargoDeps => ecosystem::cmd_migrate_cargo_deps(),
        KoboCommand::Dump { file } => run::cmd_dump(&file),
        KoboCommand::Perf {
            file,
            from,
            threshold,
            format,
            evidence,
            session,
        } => perf::cmd_perf(
            &file,
            from.as_deref(),
            threshold,
            format.as_deref(),
            &evidence,
            session.as_deref(),
        ),
        KoboCommand::Sim { command } => match command {
            SimCommand::Init {
                target,
                minimal,
                profile,
            } => sim::cmd_sim_init(&target, minimal, profile.as_deref()),
            SimCommand::Scout {
                file,
                json,
                why,
                backend_recommendations,
                fix_plan,
            } => sim::cmd_sim_scout(&file, json, why, backend_recommendations, fix_plan),
            SimCommand::Backends { json } => sim::cmd_sim_backends(json),
        },
        KoboCommand::Test {
            sim,
            profile,
            seed,
            events,
            inject,
            fuzz,
            event_budget,
            witness_dir,
            error_format,
            target,
            engine,
            backend,
            scheduler,
            max_branches,
            backend_native,
            file,
        } => test_cmd::cmd_test(
            &file,
            sim.as_deref(),
            profile.as_deref(),
            seed,
            events.as_deref(),
            inject.as_deref(),
            fuzz,
            event_budget,
            witness_dir.as_deref(),
            error_format,
            target.as_deref(),
            engine.as_deref(),
            test_cmd::BackendExpertOptions::new(backend, scheduler, max_branches, backend_native),
        ),
        KoboCommand::Replay {
            file,
            error_format,
            roundtrip_metadata,
            backend_native,
        } => replay::cmd_replay(&file, error_format, roundtrip_metadata, backend_native),
        KoboCommand::Fix {
            file,
            dry_run,
            apply,
            json,
        } => fix::cmd_fix(&file, dry_run, apply, json),
        KoboCommand::Debt {
            file,
            cargo,
            json,
            summary,
            borrows,
            patterns,
            errors,
            liveness,
            casts,
            watch,
        } => {
            if let Some(cargo_root) = cargo {
                anyhow::ensure!(
                    !(borrows || patterns || errors || liveness || casts || watch),
                    "kobo debt --cargo supports default, --summary, and --json output only"
                );
                return debt::cmd_debt_cargo(&cargo_root, json, summary);
            }
            let Some(file) = file else {
                anyhow::bail!("kobo debt requires FILE or --cargo DIR");
            };
            if casts {
                anyhow::ensure!(
                    !(borrows || patterns || errors || liveness || watch),
                    "kobo debt --casts cannot be combined with other debt views"
                );
                return debt::cmd_debt_casts(file.as_path(), json, summary);
            }
            if watch {
                return debt::cmd_debt_watch(file.as_path(), json, summary);
            }
            if borrows {
                return debt::cmd_debt_borrows(file.as_path(), json);
            }
            if patterns {
                return debt::cmd_debt_patterns(file.as_path(), json);
            }
            if errors {
                return debt::cmd_debt_errors(file.as_path(), json);
            }
            if liveness {
                return debt::cmd_debt_liveness(file.as_path(), json);
            }
            debt::cmd_debt(file.as_path(), json, summary)
        }
        KoboCommand::Init { name, from_cargo } => init::cmd_init(name.as_deref(), from_cargo),
        KoboCommand::Build {
            checked,
            strict,
            profile,
            print_policy,
            error_format,
            color,
            emit_rust,
            file,
        } => {
            let guarantee_profile = resolve_guarantee_profile(checked, strict, profile);
            build::cmd_build(
                cli_policy(guarantee_profile),
                guarantee_profile,
                print_policy,
                error_format,
                color.color_mode(),
                emit_rust,
                file.as_deref(),
            )
        }
        KoboCommand::Migrate {
            file,
            dry_run,
            apply,
            graph,
            review,
            class_view,
            explain,
            budget,
            root,
            actor,
        } => migrate::cmd_migrate(
            &file,
            dry_run,
            apply,
            graph,
            review,
            class_view,
            explain,
            budget,
            root.as_deref(),
            actor.as_deref(),
        ),
        KoboCommand::Bench { file, tick_budget } => bench::cmd_bench_tick(&file, tick_budget),
        KoboCommand::Watch {
            file,
            simple,
            build,
            plan,
            changed,
        } => watch::cmd_watch(file.as_deref(), simple, build, plan, changed.as_deref()),
        KoboCommand::Explain { code, verbose } => explain::cmd_explain(&code, verbose),
    }
}

fn cli_policy(profile: Option<GuaranteeProfileArg>) -> Option<GuaranteePolicy> {
    profile.map(|profile| GuaranteePolicy::for_profile(profile.compiler_profile()))
}
