use std::fs;
use std::path::Path;

use anyhow::Context;

pub(super) fn cmd_init(name: Option<&str>, from_cargo: bool) -> anyhow::Result<()> {
    if from_cargo {
        return cmd_init_from_cargo();
    }
    let Some(name) = name else {
        anyhow::bail!("project name is required unless --from-cargo is used");
    };
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

fn cmd_init_from_cargo() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let cargo_path = cwd.join("Cargo.toml");
    if !cargo_path.is_file() {
        anyhow::bail!("--from-cargo requires Cargo.toml in the current directory");
    }
    let cargo_source = fs::read_to_string(&cargo_path).context("failed to read Cargo.toml")?;
    let cargo: toml::Value = toml::from_str(&cargo_source).context("failed to parse Cargo.toml")?;
    let package = cargo.get("package").and_then(toml::Value::as_table);
    let name = package
        .and_then(|table| table.get("name"))
        .and_then(toml::Value::as_str)
        .unwrap_or("cargo-project");
    let version = package
        .and_then(|table| table.get("version"))
        .and_then(toml::Value::as_str)
        .unwrap_or("0.1.0");

    let kobo_path = cwd.join("Kobo.toml");
    if !kobo_path.exists() {
        let kobo_toml = format!(
            r#"[package]
name = "{name}"
version = "{version}"

[ecosystem]
default = "opaque"
replay_unknown = "debt"
"#
        );
        fs::write(&kobo_path, kobo_toml).context("failed to write Kobo.toml")?;
    }
    eprintln!("Initialized Kobo metadata from Cargo.toml");
    Ok(())
}
