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
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    commands::dispatch(args.command)
}
