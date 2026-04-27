use std::fs;
use std::path::Path;

use anyhow::Context;

pub(super) fn cmd_init(name: &str) -> anyhow::Result<()> {
    let project_dir = Path::new(name);
    if project_dir.exists() {
        anyhow::bail!("directory '{}' already exists", name);
    }

    let project_name = project_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned());

    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("failed to create {}", src_dir.display()))?;

    let kobo_toml = format!(
        r#"[package]
name = "{project_name}"
version = "0.1.0"

[kobo]
mode = "script"

[dependencies]
"#
    );
    fs::write(project_dir.join("Kobo.toml"), kobo_toml).context("failed to write Kobo.toml")?;

    fs::write(
        src_dir.join("main.kobo"),
        "fn main() {\n    println!(\"Hello from Kobo!\");\n}\n",
    )
    .context("failed to write src/main.kobo")?;

    eprintln!(
        "Created project '{project_name}' at {}",
        project_dir.display()
    );
    Ok(())
}
