use std::fs;
use std::path::{Path, PathBuf};

use crate::config::KoboConfig;
use crate::session::CompileSession;

/// Output produced by the multi-file build pipeline.
pub struct BuildOutput {
    pub gen_dir: PathBuf,
    pub rs_files: Vec<PathBuf>,
}

/// Discovers all `.kobo` files under `<project_dir>/<src_dir>`.
pub fn discover_kobo_files(project_dir: &Path, config: &KoboConfig) -> Vec<PathBuf> {
    let src_dir = project_dir.join(&config.src_dir);
    let mut files = Vec::new();
    collect_kobo_files(&src_dir, &mut files);
    files.sort();
    files
}

fn collect_kobo_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kobo_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "kobo") {
            out.push(path);
        }
    }
}

/// Generates a `Cargo.toml` for the Rust project inside `gen_dir`.
pub fn generate_cargo_toml(config: &KoboConfig, gen_dir: &Path) -> std::io::Result<()> {
    let name = if config.package_name.is_empty() {
        "kobo_project"
    } else {
        &config.package_name
    };
    let version = if config.package_version.is_empty() {
        "0.1.0"
    } else {
        &config.package_version
    };

    let mut cargo = format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"
"#
    );

    if !config.dependencies.is_empty() {
        cargo.push_str("\n[dependencies]\n");
        for (dep_name, dep_value) in &config.dependencies {
            match dep_value {
                toml::Value::String(ver) => {
                    cargo.push_str(&format!("{dep_name} = \"{ver}\"\n"));
                }
                other => {
                    cargo.push_str(&format!("{dep_name} = {other}\n"));
                }
            }
        }
    }

    fs::create_dir_all(gen_dir)?;
    fs::write(gen_dir.join("Cargo.toml"), cargo)
}

/// Runs the build pipeline: discovers .kobo files, compiles each to .rs,
/// generates Cargo.toml, and writes everything under `<project_dir>/target/kobo-gen/`.
pub fn run_build_pipeline(config: &KoboConfig, project_dir: &Path) -> Result<BuildOutput, String> {
    let gen_dir = project_dir.join("target").join("kobo-gen");
    let gen_src_dir = gen_dir.join("src");
    fs::create_dir_all(&gen_src_dir)
        .map_err(|e| format!("failed to create gen dir: {e}"))?;

    let kobo_files = discover_kobo_files(project_dir, config);
    if kobo_files.is_empty() {
        return Err(format!(
            "no .kobo files found under {}",
            project_dir.join(&config.src_dir).display()
        ));
    }

    let src_dir = project_dir.join(&config.src_dir);
    let mut rs_files = Vec::new();

    for kobo_path in &kobo_files {
        let rel = kobo_path
            .strip_prefix(&src_dir)
            .map_err(|e| format!("path error: {e}"))?;
        let rs_rel = rel.with_extension("rs");
        let rs_out = gen_src_dir.join(&rs_rel);

        if let Some(parent) = rs_out.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create dir: {e}"))?;
        }

        let mut build_config = config.clone();
        build_config.output_dir = Some(
            rs_out
                .parent()
                .unwrap_or(&gen_src_dir)
                .to_path_buf(),
        );

        let mut session = CompileSession::new(build_config);
        let rs_source = crate::pipeline::run_pipeline(&mut session, kobo_path)
            .map_err(|()| format!("compilation failed for {}", kobo_path.display()))?;

        // Don't double-write — run_pipeline already wrote via run_codegen_pipeline.
        // But the output_dir placement may differ, so write explicitly.
        fs::write(&rs_out, &rs_source)
            .map_err(|e| format!("failed to write {}: {e}", rs_out.display()))?;

        rs_files.push(rs_out);
    }

    generate_cargo_toml(config, &gen_dir)
        .map_err(|e| format!("failed to generate Cargo.toml: {e}"))?;

    Ok(BuildOutput { gen_dir, rs_files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_finds_kobo_files_in_src() {
        let tmp = std::env::temp_dir().join(format!("kobo-discover-{}", std::process::id()));
        let src = tmp.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.kobo"), "fn main() {}").unwrap();
        fs::write(src.join("lib.kobo"), "fn lib() {}").unwrap();
        fs::write(src.join("ignored.txt"), "not a kobo file").unwrap();

        let config = KoboConfig::default();
        let files = discover_kobo_files(&tmp, &config);

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("main.kobo")));
        assert!(files.iter().any(|f| f.ends_with("lib.kobo")));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn generate_cargo_toml_includes_dependencies() {
        let tmp = std::env::temp_dir().join(format!("kobo-cargo-gen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);

        let mut config = KoboConfig::default();
        config.package_name = "test_pkg".to_string();
        config.package_version = "1.0.0".to_string();
        config
            .dependencies
            .insert("log".to_string(), toml::Value::String("0.4".to_string()));

        generate_cargo_toml(&config, &tmp).unwrap();

        let content = fs::read_to_string(tmp.join("Cargo.toml")).unwrap();
        assert!(content.contains("name = \"test_pkg\""));
        assert!(content.contains("version = \"1.0.0\""));
        assert!(content.contains("log = \"0.4\""));

        let _ = fs::remove_dir_all(&tmp);
    }
}
