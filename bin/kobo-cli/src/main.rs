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
        /// [v0.5] Watch mode — re-run on file changes.
        #[arg(long, hide = true)]
        watch: bool,
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
