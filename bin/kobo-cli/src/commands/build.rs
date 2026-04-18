use anyhow::Context;
use kobo_driver::{load_config, KoboMode};

pub(super) fn cmd_build(cli_mode: Option<KoboMode>) -> anyhow::Result<()> {
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
