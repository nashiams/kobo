mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    },
    /// Compile and print the generated .rs to stdout without invoking rustc.
    Inspect {
        #[arg(value_name = "FILE")]
        file: PathBuf,
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
        /// [v0.5] Watch mode — re-run on file changes.
        #[arg(long, hide = true)]
        watch: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    commands::dispatch(args.command)
}
