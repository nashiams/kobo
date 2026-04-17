use anyhow::Context;
use kobo_driver::{load_config, KoboMode};

pub(super) fn cmd_build(cli_mode: Option<KoboMode>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let mut config = load_config(&cwd)?;

    if let Some(mode) = cli_mode {
        config.mode = mode;
    }

    kobo_driver::run_build_pipeline(&config, &cwd)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}
