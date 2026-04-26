mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use kobo_ir::KoboMode;

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
        #[arg(long, conflicts_with = "strict",
              help = "Check in checked mode — ownership advisory warnings")]
        checked: bool,
        #[arg(long, conflicts_with = "checked",
              help = "Check in strict mode (v0.9 — not yet implemented)")]
        strict: bool,
        #[arg(long,
              help = "Show full solver pipeline diagnostics (constraint graph, clusters, solver outcome)")]
        pipeline: bool,
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
        #[arg(long, conflicts_with = "strict",
              help = "Run in checked mode — ownership advisory warnings")]
        checked: bool,
        #[arg(long, conflicts_with = "checked",
              help = "Run in strict mode (v0.9 — not yet implemented)")]
        strict: bool,
    },
    /// Compile and print the generated .rs to stdout without invoking rustc.
    Inspect {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(long, conflicts_with = "strict",
              help = "Inspect in checked mode — ownership advisory warnings")]
        checked: bool,
        #[arg(long, conflicts_with = "checked",
              help = "Inspect in strict mode (v0.9 — not yet implemented)")]
        strict: bool,
        #[arg(long,
              help = "Strip all Kobo wrappers, output standalone Rust")]
        clean: bool,
        #[arg(long,
              help = "Erase lifetime parameters in script mode (S-21: auto-own references)")]
        erase_lifetimes: bool,
        #[arg(long, value_name = "DIR",
              help = "Generate a complete Cargo project to DIR")]
        cargo: Option<PathBuf>,
    },
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
    },
    /// Show ownership debt report for a .kobo file.
    Debt {
        #[arg(value_name = "FILE")]
        file: PathBuf,
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
        /// Apply migration changes to the source file
        #[arg(long)]
        apply: bool,
        /// Show dependency graph
        #[arg(long)]
        graph: bool,
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
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Build all .kobo files in a Kobo project.
    Build {
        #[arg(long, conflicts_with = "strict",
              help = "Build in checked mode")]
        checked: bool,
        #[arg(long, conflicts_with = "checked",
              help = "Build in strict mode")]
        strict: bool,
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
}

/// Resolve CLI mode flags to a KoboMode override.
/// Returns None when no CLI flag is present — the config file (Kobo.toml) or
/// default (Script) is used instead [Contract R06: CLI overrides per-crate mode].
pub(crate) fn resolve_cli_mode(checked: bool, strict: bool) -> Option<KoboMode> {
    if strict {
        Some(KoboMode::Strict)
    } else if checked {
        Some(KoboMode::Checked)
    } else {
        None
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    commands::dispatch(args.command)
}
