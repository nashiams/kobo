mod commands;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use kobo_errors::ColorMode;
use kobo_ir::GuaranteeProfile;

#[derive(Parser, Debug)]
#[command(name = "kobo", about = "The Kobo compiler")]
struct Args {
    #[command(subcommand)]
    command: KoboCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum KoboCommand {
    /// Analyze a .kobo file without generating or running a binary.
    Check {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(
            long,
            conflicts_with = "strict",
            help = "Compatibility alias for --profile checked"
        )]
        checked: bool,
        #[arg(
            long,
            conflicts_with = "checked",
            help = "Compatibility alias for --profile release"
        )]
        strict: bool,
        #[arg(long, value_enum, help = "Select a guarantee policy preset")]
        profile: Option<GuaranteeProfileArg>,
        #[arg(
            long = "print-policy",
            value_enum,
            help = "Print expanded guarantee policy"
        )]
        print_policy: Option<PolicyOutputFormat>,
        #[arg(
            long,
            help = "Show full solver pipeline diagnostics (constraint graph, clusters, solver outcome)"
        )]
        pipeline: bool,
        #[arg(long = "error-format", value_enum, default_value_t = ErrorFormat::Human)]
        error_format: ErrorFormat,
        #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
        color: ColorArg,
        #[arg(
            long,
            help = "Recover from parser errors and continue trustworthy phases"
        )]
        recover_parse: bool,
        #[arg(long, help = "Treat external boundaries as future replay-critical")]
        replay_critical: bool,
        #[arg(
            long,
            value_name = "N",
            help = "Limit visible diagnostics in human/editor output"
        )]
        max_diagnostics: Option<usize>,
        #[arg(
            long,
            value_name = "START-END",
            help = "Prioritize diagnostics in a visible line range"
        )]
        visible_region: Option<String>,
        #[arg(long, help = "Keep budgeted diagnostics in machine output")]
        include_budgeted: bool,
    },
    /// Export diagnostics in the LSP diagnostic shape.
    LspDiagnostics {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = ErrorFormat::Json)]
        format: ErrorFormat,
        #[arg(long, help = "Allow diagnostics for a file outside a Kobo project")]
        no_project_ok: bool,
        #[arg(
            long,
            alias = "actions",
            help = "Include action metadata for editor clients"
        )]
        include_actions: bool,
    },
    /// Reformat a .kobo file when the source map proves the edit is lossless.
    Fmt {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Compile and run a .kobo file.
    Run {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(
            long,
            conflicts_with = "strict",
            help = "Compatibility alias for --profile checked"
        )]
        checked: bool,
        #[arg(
            long,
            conflicts_with = "checked",
            help = "Compatibility alias for --profile release"
        )]
        strict: bool,
        #[arg(long, value_enum, help = "Select a guarantee policy preset")]
        profile: Option<GuaranteeProfileArg>,
        #[arg(
            long,
            help = "Erase lifetime parameters before rustc for dev or checked profiles"
        )]
        erase_lifetimes: bool,
    },
    /// Compile and print the generated .rs to stdout without invoking rustc.
    Inspect {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(
            long,
            conflicts_with = "strict",
            help = "Compatibility alias for --profile checked"
        )]
        checked: bool,
        #[arg(
            long,
            conflicts_with = "checked",
            help = "Compatibility alias for --profile release"
        )]
        strict: bool,
        #[arg(long, help = "Strip all Kobo wrappers, output standalone Rust")]
        clean: bool,
        #[arg(
            long,
            help = "Erase lifetime parameters for dev-profile auto-own references"
        )]
        erase_lifetimes: bool,
        #[arg(long, help = "Show indexed scenario metadata")]
        scenario_metadata: bool,
        #[arg(long, help = "Show simulation transparency metadata")]
        sim: bool,
        #[arg(
            long,
            requires = "sim",
            help = "Reserve generated simulation harness transparency output"
        )]
        harness: bool,
        #[arg(
            long,
            value_name = "DIR",
            help = "Generate a complete Cargo project to DIR"
        )]
        cargo: Option<PathBuf>,
        #[arg(
            long,
            value_name = "PROFILE",
            help = "Select an advisory stack profile"
        )]
        profile: Option<String>,
        #[arg(
            long = "trait-default",
            value_name = "MODE",
            help = "Select trait facade lowering default"
        )]
        trait_default: Option<String>,
        #[arg(long, value_name = "FORMAT", help = "Inspect ownership audit evidence")]
        audit: Option<String>,
    },
    /// Advisory project dependency and profile report.
    Doctor {
        #[arg(long, help = "Inspect Cargo dependency shape")]
        deps: bool,
        #[arg(long, help = "Report self-host compatibility readiness")]
        self_host: bool,
        #[arg(long, help = "Emit JSON")]
        json: bool,
    },
    /// Generate a draft Kobo declaration file from a Rust crate.
    Bindgen {
        #[arg(long = "crate", value_name = "CRATE")]
        crate_option: Option<String>,
        #[arg(long, value_name = "LIST")]
        features: Option<String>,
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Add a Cargo dependency without requiring Kobo metadata packages.
    Add {
        #[arg(value_name = "CRATE")]
        crate_name: String,
        #[arg(long, value_name = "LIST")]
        features: Option<String>,
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,
        #[arg(long, value_name = "URL")]
        git: Option<String>,
        #[arg(long = "no-default-features")]
        no_default_features: bool,
        #[arg(long, help = "Add to [dev-dependencies]")]
        dev: bool,
        #[arg(long, help = "Add to [build-dependencies]")]
        build: bool,
        #[arg(
            long,
            value_name = "TARGET",
            help = "Add to target-specific dependencies"
        )]
        target: Option<String>,
        #[arg(long, value_name = "PATH")]
        manifest_path: Option<PathBuf>,
        #[arg(long, value_name = "MEMBER")]
        member: Option<String>,
    },
    /// Record an optional kobo-types package for a dependency.
    AddTypes {
        #[arg(value_name = "CRATE")]
        crate_name: String,
    },
    /// Record an optional Kobo adapter package for a dependency.
    AddAdapter {
        #[arg(value_name = "CRATE")]
        crate_name: String,
    },
    /// Seed Kobo ecosystem metadata from Cargo dependencies.
    MigrateCargoDeps,
    /// Run the pipeline through the KIR phase only and print KIR nodes.
    Dump {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Show hot borrow paths from a prior `KOBO_DIAG=1` run.
    Perf {
        /// Path to source file for context (used in run suggestions).
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Path to stderr capture from `KOBO_DIAG=1 kobo run FILE 2>diag.log`
        #[arg(long, value_name = "FILE")]
        from: Option<PathBuf>,
        /// Override display threshold (default: KOBO_DIAG_THRESHOLD env or 10000)
        #[arg(long)]
        threshold: Option<u64>,
        /// Output format for source-derived performance evidence.
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Select estimate or structured production evidence.
        #[arg(long, value_name = "KIND", default_value = "estimate")]
        evidence: String,
        /// Structured evidence session directory for --evidence real.
        #[arg(long, value_name = "DIR")]
        session: Option<PathBuf>,
    },
    /// Simulation and replay evidence helpers.
    Sim {
        #[command(subcommand)]
        command: SimCommand,
    },
    /// Run reserved v0.9 test/replay gates.
    Test {
        #[arg(long, value_name = "PROFILE")]
        sim: Option<String>,
        #[arg(long, value_name = "PROFILE")]
        profile: Option<String>,
        #[arg(long, value_name = "SEED")]
        seed: Option<u64>,
        #[arg(long, value_name = "FORMAT")]
        events: Option<String>,
        #[arg(long, value_name = "HOOKS")]
        inject: Option<String>,
        #[arg(long, help = "Run deterministic generated stateful-input cases")]
        fuzz: bool,
        #[arg(long = "event-budget", value_name = "N")]
        event_budget: Option<u64>,
        #[arg(long = "witness-dir", value_name = "DIR")]
        witness_dir: Option<PathBuf>,
        #[arg(long = "error-format", value_enum, default_value_t = ErrorFormat::Human)]
        error_format: ErrorFormat,
        #[arg(long, value_name = "SCENARIO")]
        target: Option<String>,
        #[arg(long, value_name = "ENGINE")]
        engine: Option<String>,
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Validate and summarize a .kwit witness without executing replay.
    Replay {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(long = "error-format", value_enum, default_value_t = ErrorFormat::Human)]
        error_format: ErrorFormat,
        #[arg(long)]
        roundtrip_metadata: bool,
    },
    /// Apply safe machine-applicable codemods.
    Fix {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show ownership debt report for a .kobo file.
    Debt {
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
        /// Scan a standalone Rust Cargo project without requiring Kobo sources.
        #[arg(long, value_name = "DIR")]
        cargo: Option<PathBuf>,
        /// Output JSON (schema_version=1, stable from v0.4)
        #[arg(long)]
        json: bool,
        /// Output a single summary line
        #[arg(long)]
        summary: bool,
        /// Show borrow overlap analysis
        #[arg(long)]
        borrows: bool,
        /// Show migration patterns (Rc→Arc, clone elimination, etc.)
        #[arg(long)]
        patterns: bool,
        /// Show error-handling debt (boxed/dynamic error → typed enum opportunities)
        #[arg(long)]
        errors: bool,
        /// Show must_call liveness debt
        #[arg(long)]
        liveness: bool,
        /// [v0.5] Watch mode — re-run on file changes.
        #[arg(long, hide = true)]
        watch: bool,
    },
    /// Migrate ownership tiers in a .kobo file.
    Migrate {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Show diff without applying changes (default)
        #[arg(long)]
        dry_run: bool,
        /// Apply supported safe source rewrites in place
        #[arg(long, hide = true)]
        apply: bool,
        /// Show dependency graph
        #[arg(long)]
        graph: bool,
        /// Show candidate review details for non-unique solver outcomes
        #[arg(long)]
        review: bool,
        /// Show decision class/profile summary
        #[arg(long = "class")]
        class_view: bool,
        /// Show solver explanations in dry-run output
        #[arg(long)]
        explain: bool,
        /// Override solver budget in seconds
        #[arg(long, value_name = "SECONDS")]
        budget: Option<f64>,
        /// Restrict migration scope to a specific function
        #[arg(long, value_name = "FN_NAME")]
        root: Option<String>,
        /// Generate actor scaffold at FILE:LINE
        #[arg(long, value_name = "SPEC")]
        actor: Option<String>,
    },
    /// Create a new Kobo project skeleton.
    Init {
        /// Name (and directory) for the new project.
        #[arg(value_name = "NAME", required_unless_present = "from_cargo")]
        name: Option<String>,
        /// Create Kobo metadata for an existing Cargo project.
        #[arg(long)]
        from_cargo: bool,
    },
    /// Build all .kobo files in a Kobo project.
    Build {
        #[arg(
            long,
            conflicts_with = "strict",
            help = "Compatibility alias for --profile checked"
        )]
        checked: bool,
        #[arg(
            long,
            conflicts_with = "checked",
            help = "Compatibility alias for --profile release"
        )]
        strict: bool,
        #[arg(long, value_enum, help = "Select a guarantee policy preset")]
        profile: Option<GuaranteeProfileArg>,
        #[arg(
            long = "print-policy",
            value_enum,
            help = "Print expanded guarantee policy"
        )]
        print_policy: Option<PolicyOutputFormat>,
        #[arg(long = "error-format", value_enum, default_value_t = ErrorFormat::Human)]
        error_format: ErrorFormat,
        #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
        color: ColorArg,
        #[arg(long, help = "Print generated Rust for a single input file")]
        emit_rust: bool,
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
    },
    /// Per-function timing report for tick loop functions.
    Bench {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Report tick budget usage
        #[arg(long)]
        tick_budget: bool,
    },
    /// File-watcher re-run on save.
    Watch {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Simple mode: save → compile → run (no state persistence)
        #[arg(long)]
        simple: bool,
        /// Build mode: save → codegen → cargo build (full rebuild cycle)
        #[arg(long)]
        build: bool,
    },
    /// Explain a Kobo diagnostic code.
    Explain {
        #[arg(value_name = "CODE")]
        code: String,
        #[arg(long, help = "Show registry metadata and machine policy details")]
        verbose: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SimCommand {
    /// Create a minimal simulation island descriptor for one target.
    Init {
        #[arg(long, value_name = "FILE:SYMBOL")]
        target: String,
        #[arg(long)]
        minimal: bool,
        #[arg(long, value_name = "PROFILE")]
        profile: Option<String>,
    },
    /// Rank likely first simulation evidence targets.
    Scout {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        why: bool,
        #[arg(long)]
        backend_recommendations: bool,
        #[arg(long)]
        fix_plan: bool,
    },
    /// List deterministic-testing backend metadata.
    Backends {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ErrorFormat {
    Human,
    Json,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ColorArg {
    Auto,
    Always,
    Never,
}

impl ColorArg {
    pub(crate) const fn color_mode(self) -> ColorMode {
        match self {
            Self::Auto => ColorMode::Auto,
            Self::Always => ColorMode::Always,
            Self::Never => ColorMode::Never,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum GuaranteeProfileArg {
    Dev,
    Checked,
    Release,
}

impl GuaranteeProfileArg {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Checked => "checked",
            Self::Release => "release",
        }
    }

    pub(crate) const fn compiler_profile(self) -> GuaranteeProfile {
        match self {
            Self::Dev => GuaranteeProfile::Dev,
            Self::Checked => GuaranteeProfile::Checked,
            Self::Release => GuaranteeProfile::Release,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum PolicyOutputFormat {
    Json,
}

/// Resolve public profile controls to the selected guarantee profile.
///
/// `--checked` and `--strict` are kept only as compatibility aliases for users
/// who have not migrated to `--profile checked|release` yet.
pub(crate) fn resolve_guarantee_profile(
    checked: bool,
    strict: bool,
    profile: Option<GuaranteeProfileArg>,
) -> Option<GuaranteeProfileArg> {
    if strict {
        Some(GuaranteeProfileArg::Release)
    } else if checked {
        Some(GuaranteeProfileArg::Checked)
    } else {
        profile
    }
}

fn main() -> std::process::ExitCode {
    match std::thread::Builder::new()
        .name("kobo-main".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_main)
    {
        Ok(handle) => match handle.join() {
            Ok(code) => code,
            Err(_) => {
                eprintln!("Error: kobo command thread panicked");
                std::process::ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("Error: failed to start kobo command thread: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_main() -> std::process::ExitCode {
    let args = Args::parse_from(normalized_args());
    match commands::dispatch(args.command) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) if commands::is_diagnostic_exit(&error) => std::process::ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Error: {error:?}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn normalized_args() -> Vec<OsString> {
    let mut args = std::env::args_os().collect::<Vec<_>>();
    if args.get(1).is_some_and(|arg| arg == OsStr::new("bindgen"))
        && args.get(2).is_some_and(|arg| !starts_with_dash(arg))
    {
        args.insert(2, OsString::from("--crate"));
    }
    args
}

fn starts_with_dash(value: &OsStr) -> bool {
    value.to_str().is_some_and(|value| value.starts_with('-'))
}
