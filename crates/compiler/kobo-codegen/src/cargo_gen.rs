/// Generate a complete Cargo project directory from Kobo source.
///
/// Structure:
///   output_dir/
///   ├── Cargo.toml          # from Kobo.toml [package] + [dependencies]
///   ├── src/
///   │   ├── main.rs         # from src/main.kobo (clean-stripped)
///   │   └── <module>.rs     # from src/<module>.kobo (clean-stripped)
///   └── .gitignore          # standard Rust gitignore
///
/// Cargo.toml generation:
/// - [package] from Kobo.toml [package]
/// - [dependencies] from Kobo.toml [dependencies]
/// - edition = "2021" (default)
/// - NO kobo_diag dependency
/// - NO kobo_ prefixed dependencies
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CargoGenError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Kobo.toml parse error: {0}")]
    TomlParse(String),
}

/// Minimal Kobo project config for cargo generation.
#[derive(Debug, Clone)]
pub struct KoboProjectConfig {
    pub name: String,
    pub version: String,
    pub edition: String,
    /// Non-kobo dependencies as `(name, version_req)` pairs.
    pub dependencies: Vec<(String, String)>,
}

impl Default for KoboProjectConfig {
    fn default() -> Self {
        Self {
            name: "my-project".to_owned(),
            version: "0.1.0".to_owned(),
            edition: "2021".to_owned(),
            dependencies: Vec::new(),
        }
    }
}

impl KoboProjectConfig {
    /// Parse from a Kobo.toml string.
    pub fn from_toml(toml_str: &str) -> Result<Self, CargoGenError> {
        let table: toml::Table = toml_str
            .parse()
            .map_err(|e: toml::de::Error| CargoGenError::TomlParse(e.to_string()))?;

        let package = table.get("package").and_then(|v| v.as_table());
        let name = package
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("my-project")
            .to_owned();
        let version = package
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0")
            .to_owned();
        let edition = package
            .and_then(|p| p.get("edition"))
            .and_then(|v| v.as_str())
            .unwrap_or("2021")
            .to_owned();

        let mut dependencies = Vec::new();
        if let Some(deps) = table.get("dependencies").and_then(|v| v.as_table()) {
            for (dep_name, dep_val) in deps {
                // Skip kobo_ prefixed dependencies.
                if dep_name.starts_with("kobo") {
                    continue;
                }
                let version_str = dependency_manifest_spec(dep_val);
                dependencies.push((dep_name.clone(), version_str));
            }
        }

        Ok(Self {
            name,
            version,
            edition,
            dependencies,
        })
    }
}

/// Generate a complete Cargo project directory.
pub fn generate_cargo_project(
    config: &KoboProjectConfig,
    source_files: &[(PathBuf, String)], // (kobo path, clean source)
    output_dir: &Path,
) -> Result<(), CargoGenError> {
    let config = config_with_inferred_dependencies(config, source_files);

    // Create output directory structure.
    let src_dir = output_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    // Generate Cargo.toml.
    let cargo_toml = generate_cargo_toml(&config);
    fs::write(output_dir.join("Cargo.toml"), cargo_toml)?;

    // Generate .gitignore.
    fs::write(output_dir.join(".gitignore"), "/target\n")?;

    // Write source files.
    for (kobo_path, clean_source) in source_files {
        let rs_path = if source_files.len() == 1 {
            PathBuf::from("main.rs")
        } else {
            rs_relative_path(kobo_path)
        };
        let output_path = src_dir.join(rs_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, clean_source)?;
    }

    Ok(())
}

fn dependency_manifest_spec(dep_val: &toml::Value) -> String {
    match dep_val {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(table) => format_inline_table(table),
        other => other.to_string(),
    }
}

fn format_inline_table(table: &toml::map::Map<String, toml::Value>) -> String {
    let entries = table
        .iter()
        .map(|(key, value)| format!("{key} = {}", format_toml_value(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {entries} }}")
}

fn format_toml_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        toml::Value::Integer(_)
        | toml::Value::Float(_)
        | toml::Value::Boolean(_)
        | toml::Value::Datetime(_) => value.to_string(),
        toml::Value::Array(values) => {
            let inner = values
                .iter()
                .map(format_toml_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        toml::Value::Table(table) => format_inline_table(table),
    }
}

fn rs_relative_path(kobo_path: &Path) -> PathBuf {
    let mut rel = kobo_path
        .strip_prefix("src")
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| kobo_path.to_path_buf());
    rel.set_extension("rs");
    rel
}

fn generate_cargo_toml(config: &KoboProjectConfig) -> String {
    let mut toml = String::new();
    toml.push_str("[package]\n");
    toml.push_str(&format!("name = \"{}\"\n", config.name));
    toml.push_str(&format!("version = \"{}\"\n", config.version));
    toml.push_str(&format!("edition = \"{}\"\n", config.edition));
    toml.push('\n');

    if !config.dependencies.is_empty() {
        toml.push_str("[dependencies]\n");
        for (dep_name, dep_version) in &config.dependencies {
            if dep_version.trim_start().starts_with('{') {
                toml.push_str(&format!("{dep_name} = {dep_version}\n"));
            } else {
                toml.push_str(&format!("{dep_name} = \"{dep_version}\"\n"));
            }
        }
        toml.push('\n');
    }

    toml.push_str("[workspace]\n");

    toml
}

fn config_with_inferred_dependencies(
    config: &KoboProjectConfig,
    source_files: &[(PathBuf, String)],
) -> KoboProjectConfig {
    let mut config = config.clone();
    let needs_tokio = source_files
        .iter()
        .any(|(_, source)| source.contains("tokio::"));
    let has_tokio = config.dependencies.iter().any(|(name, _)| name == "tokio");

    if needs_tokio && !has_tokio {
        config.dependencies.push((
            "tokio".to_owned(),
            "{ version = \"1\", features = [\"rt-multi-thread\", \"macros\", \"sync\", \"time\"] }"
                .to_owned(),
        ));
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn generate_project_structure() {
        let config = KoboProjectConfig {
            name: "test-project".to_owned(),
            version: "0.1.0".to_owned(),
            edition: "2021".to_owned(),
            dependencies: vec![],
        };
        let sources = vec![(
            PathBuf::from("src/main.kobo"),
            "fn main() { println!(\"hello\"); }".to_owned(),
        )];

        let temp = tempfile::tempdir().unwrap();
        generate_cargo_project(&config, &sources, temp.path()).unwrap();

        assert!(temp.path().join("Cargo.toml").exists());
        assert!(temp.path().join("src/main.rs").exists());
        assert!(temp.path().join(".gitignore").exists());

        let cargo_toml = fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("test-project"));
        assert!(cargo_toml.contains("[workspace]"));
        assert!(!cargo_toml.contains("kobo"));
    }

    #[test]
    fn cargo_toml_has_dependencies() {
        let config = KoboProjectConfig {
            name: "my-app".to_owned(),
            version: "1.0.0".to_owned(),
            edition: "2021".to_owned(),
            dependencies: vec![
                ("tokio".to_owned(), "1".to_owned()),
                ("serde".to_owned(), "1".to_owned()),
            ],
        };

        let toml = generate_cargo_toml(&config);
        assert!(toml.contains("tokio = \"1\""));
        assert!(toml.contains("serde = \"1\""));
    }

    #[test]
    fn raw_dependency_specs_are_not_quoted() {
        let config = KoboProjectConfig {
            name: "raw-deps".to_owned(),
            version: "0.1.0".to_owned(),
            edition: "2021".to_owned(),
            dependencies: vec![(
                "tokio".to_owned(),
                "{ version = \"1\", features = [\"time\"] }".to_owned(),
            )],
        };

        let toml = generate_cargo_toml(&config);
        assert!(toml.contains("tokio = { version = \"1\", features = [\"time\"] }"));
    }

    #[test]
    fn cargo_generation_infers_tokio_dependency_from_generated_source() {
        let config = KoboProjectConfig::default();
        let sources = vec![(
            PathBuf::from("src/main.kobo"),
            "fn main() { let _ = tokio::time::interval; }".to_owned(),
        )];

        let temp = tempfile::tempdir().unwrap();
        generate_cargo_project(&config, &sources, temp.path()).unwrap();

        let cargo_toml = fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("tokio = { version = \"1\""));
        assert!(cargo_toml.contains("\"time\""));
    }

    #[test]
    fn parse_kobo_toml_preserves_table_dependency_features() {
        let toml_str = r#"
[package]
name = "async-app"
version = "0.1.0"

[dependencies]
tokio = { version = "1", features = ["rt", "macros", "sync", "time"] }
"#;
        let config = KoboProjectConfig::from_toml(toml_str).unwrap();
        let dep = config
            .dependencies
            .iter()
            .find(|(name, _)| name == "tokio")
            .expect("tokio dependency should be preserved");

        assert!(dep.1.starts_with('{'));
        assert!(dep.1.contains("version = \"1\""));
        assert!(dep
            .1
            .contains("features = [\"rt\", \"macros\", \"sync\", \"time\"]"));
    }

    #[test]
    fn default_config_generates_valid_package_metadata() {
        let config = KoboProjectConfig::default();
        let cargo_toml = generate_cargo_toml(&config);

        assert!(cargo_toml.contains("name = \"my-project\""));
        assert!(cargo_toml.contains("version = \"0.1.0\""));
        assert!(cargo_toml.contains("edition = \"2021\""));
    }

    #[test]
    fn single_source_project_uses_main_rs_entrypoint() {
        let config = KoboProjectConfig::default();
        let sources = vec![(
            PathBuf::from("fixtures/uc1_video_pipeline.kobo"),
            "fn main() {}".to_owned(),
        )];

        let temp = tempfile::tempdir().unwrap();
        generate_cargo_project(&config, &sources, temp.path()).unwrap();

        assert!(temp.path().join("src/main.rs").exists());
        assert!(!temp.path().join("src/uc1_video_pipeline.rs").exists());
    }

    #[test]
    fn parse_kobo_toml_filters_kobo_deps() {
        let toml_str = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
tokio = "1"
serde = { version = "1", features = ["derive"] }
kobo-diag = "0.1"
kobo-runtime = { path = "../kobo-runtime" }
"#;
        let config = KoboProjectConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.name, "my-app");
        assert_eq!(config.dependencies.len(), 2);
        assert!(config.dependencies.iter().any(|(n, _)| n == "tokio"));
        assert!(config.dependencies.iter().any(|(n, _)| n == "serde"));
        assert!(!config
            .dependencies
            .iter()
            .any(|(n, _)| n.starts_with("kobo")));
    }

    #[test]
    fn source_files_written_as_rs() {
        let config = KoboProjectConfig {
            name: "multi".to_owned(),
            version: "0.1.0".to_owned(),
            edition: "2021".to_owned(),
            dependencies: vec![],
        };
        let sources = vec![
            (PathBuf::from("src/main.kobo"), "fn main() {}".to_owned()),
            (
                PathBuf::from("src/lib.kobo"),
                "pub fn greet() {}".to_owned(),
            ),
        ];

        let temp = tempfile::tempdir().unwrap();
        generate_cargo_project(&config, &sources, temp.path()).unwrap();

        assert!(temp.path().join("src/main.rs").exists());
        assert!(temp.path().join("src/lib.rs").exists());

        let lib_content = fs::read_to_string(temp.path().join("src/lib.rs")).unwrap();
        assert!(lib_content.contains("pub fn greet()"));
    }

    #[test]
    fn multi_source_project_preserves_nested_module_paths() {
        let config = KoboProjectConfig::default();
        let sources = vec![
            (PathBuf::from("src/main.kobo"), "mod domain;".to_owned()),
            (
                PathBuf::from("src/domain/mod.kobo"),
                "pub mod order;".to_owned(),
            ),
            (
                PathBuf::from("src/domain/order.kobo"),
                "pub struct Order;".to_owned(),
            ),
        ];

        let temp = tempfile::tempdir().unwrap();
        generate_cargo_project(&config, &sources, temp.path()).unwrap();

        assert!(temp.path().join("src/main.rs").exists());
        assert!(temp.path().join("src/domain/mod.rs").exists());
        assert!(temp.path().join("src/domain/order.rs").exists());
    }
}
