/// Generate a complete Cargo project directory from Kobo source.
///
/// Structure:
///   output_dir/
///   ├── Cargo.toml          # from Kobo.toml [package] + [dependencies]
///   ├── src/
///   │   ├── main.rs         # from src/main.kobo (clean-stripped)
///   │   └── <module>.rs     # from src/<module>.kobo (clean-stripped)
///   └──.gitignore          # standard Rust gitignore
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

use crate::KoboSourceMap;

#[derive(Debug, Error)]
pub enum CargoGenError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Kobo.toml parse error: {0}")]
    TomlParse(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Minimal Kobo project config for cargo generation.
#[derive(Debug, Clone)]
pub struct KoboProjectConfig {
    pub name: String,
    pub version: String,
    pub edition: String,
    /// Non-kobo dependencies as `(name, version_req)` pairs.
    pub dependencies: Vec<(String, String)>,
    pub dev_dependencies: Vec<(String, String)>,
    pub build_dependencies: Vec<(String, String)>,
    pub target_dependencies: Vec<TargetDependencyConfig>,
}

#[derive(Debug, Clone)]
pub struct TargetDependencyConfig {
    pub target: String,
    pub dependencies: Vec<(String, String)>,
    pub dev_dependencies: Vec<(String, String)>,
    pub build_dependencies: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct CargoSourceFile {
    pub kobo_path: PathBuf,
    pub clean_source: String,
    pub source_map: Option<KoboSourceMap>,
}

impl CargoSourceFile {
    pub fn without_source_map(kobo_path: PathBuf, clean_source: String) -> Self {
        Self {
            kobo_path,
            clean_source,
            source_map: None,
        }
    }
}

impl Default for KoboProjectConfig {
    fn default() -> Self {
        Self {
            name: "my-project".to_owned(),
            version: "0.1.0".to_owned(),
            edition: "2021".to_owned(),
            dependencies: Vec::new(),
            dev_dependencies: Vec::new(),
            build_dependencies: Vec::new(),
            target_dependencies: Vec::new(),
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

        let dependencies = dependency_section(&table, "dependencies");
        let dev_dependencies = dependency_section(&table, "dev-dependencies");
        let build_dependencies = dependency_section(&table, "build-dependencies");
        let target_dependencies = target_dependency_sections(&table);

        Ok(Self {
            name,
            version,
            edition,
            dependencies,
            dev_dependencies,
            build_dependencies,
            target_dependencies,
        })
    }
}

fn dependency_section(table: &toml::Table, section: &str) -> Vec<(String, String)> {
    let mut dependencies = Vec::new();
    if let Some(deps) = table.get(section).and_then(|v| v.as_table()) {
        for (dep_name, dep_val) in deps {
            if dep_name.starts_with("kobo") {
                continue;
            }
            let version_str = dependency_manifest_spec(dep_val);
            dependencies.push((dep_name.clone(), version_str));
        }
    }
    dependencies.sort_by(|left, right| left.0.cmp(&right.0));
    dependencies
}

fn target_dependency_sections(table: &toml::Table) -> Vec<TargetDependencyConfig> {
    let mut sections = Vec::new();
    let Some(targets) = table.get("target").and_then(|value| value.as_table()) else {
        return sections;
    };
    for (target, value) in targets {
        let Some(target_table) = value.as_table() else {
            continue;
        };
        let dependencies = dependency_section(target_table, "dependencies");
        let dev_dependencies = dependency_section(target_table, "dev-dependencies");
        let build_dependencies = dependency_section(target_table, "build-dependencies");
        if !dependencies.is_empty()
            || !dev_dependencies.is_empty()
            || !build_dependencies.is_empty()
        {
            sections.push(TargetDependencyConfig {
                target: target.clone(),
                dependencies,
                dev_dependencies,
                build_dependencies,
            });
        }
    }
    sections.sort_by(|left, right| left.target.cmp(&right.target));
    sections
}

/// Generate a complete Cargo project directory.
pub fn generate_cargo_project(
    config: &KoboProjectConfig,
    source_files: &[(PathBuf, String)], // (kobo path, clean source)
    output_dir: &Path,
) -> Result<(), CargoGenError> {
    let source_files = source_files
        .iter()
        .map(|(kobo_path, clean_source)| {
            CargoSourceFile::without_source_map(kobo_path.clone(), clean_source.clone())
        })
        .collect::<Vec<_>>();
    generate_cargo_project_with_maps(config, &source_files, output_dir)
}

pub fn generate_cargo_project_with_maps(
    config: &KoboProjectConfig,
    source_files: &[CargoSourceFile],
    output_dir: &Path,
) -> Result<(), CargoGenError> {
    let config = config_with_inferred_dependencies(config, source_files);

    // Create output directory structure.
    let src_dir = output_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    // Generate Cargo.toml.
    let cargo_toml = generate_cargo_toml(&config);
    fs::write(output_dir.join("Cargo.toml"), cargo_toml)?;

    // Generate.gitignore.
    fs::write(output_dir.join(".gitignore"), "/target\n")?;

    // Write source files.
    for source_file in source_files {
        let rs_path = if source_files.len() == 1 {
            PathBuf::from("main.rs")
        } else {
            rs_relative_path(&source_file.kobo_path)
        };
        let output_path = src_dir.join(rs_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, &source_file.clean_source)?;
        write_source_map_if_present(source_file, source_files.len(), &src_dir)?;
    }

    write_backend_manifest(output_dir, &config, source_files)?;
    Ok(())
}

fn write_source_map_if_present(
    source_file: &CargoSourceFile,
    source_count: usize,
    src_dir: &Path,
) -> Result<(), CargoGenError> {
    let Some(source_map) = source_file.source_map.as_ref() else {
        return Ok(());
    };
    let map_path = src_dir.join(source_map_relative_path(
        &source_file.kobo_path,
        source_count,
    ));
    if let Some(parent) = map_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(map_path, serde_json::to_vec_pretty(source_map)?)?;
    Ok(())
}

fn write_backend_manifest(
    output_dir: &Path,
    config: &KoboProjectConfig,
    source_files: &[CargoSourceFile],
) -> Result<(), CargoGenError> {
    let manifest_dir = output_dir.join(".kobo");
    fs::create_dir_all(&manifest_dir)?;
    fs::write(
        manifest_dir.join("generated-backend.json"),
        serde_json::to_vec_pretty(&backend_manifest_json(config, source_files))?,
    )?;
    Ok(())
}

fn backend_manifest_json(
    config: &KoboProjectConfig,
    source_files: &[CargoSourceFile],
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "backend": "generated-rust",
        "source_of_truth": "kobo",
        "role": "generated Rust is a backend, not the source of truth",
        "debug_workflows": ["inspect_clean_cargo", "cargo_check", "source_map_diagnostics", "replay_debug"],
        "release_workflows": ["cargo_check", "cargo_clippy", "cargo_test", "source_map_diagnostics", "replay_debug"],
        "target_matrix": target_matrix_json(config),
        "project_concepts": ["diagnostics", "replay", "debt", "proof", "lsp"],
        "files": source_files
            .iter()
            .map(|source_file| {
                let rust_output = if source_files.len() == 1 {
                    PathBuf::from("src").join("main.rs")
                } else {
                    PathBuf::from("src").join(rs_relative_path(&source_file.kobo_path))
                };
                serde_json::json!({
                    "kobo_source": normalized_display(&source_file.kobo_path),
                    "rust_output": normalized_display(&rust_output),
                    "source_map": normalized_display(
                        &PathBuf::from("src")
                            .join(source_map_relative_path(&source_file.kobo_path, source_files.len())),
                    ),
                    "generated_rust_role": "backend",
                    "source_of_truth": "kobo",
                    "project_concepts": ["diagnostics", "replay", "debt", "proof", "lsp"],
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn target_matrix_json(config: &KoboProjectConfig) -> serde_json::Value {
    serde_json::json!({
        "status": target_matrix_status(config),
        "package": {
            "name": config.name,
            "version": config.version,
            "edition": config.edition,
        },
        "host": {
            "dependencies": dependency_entries(&config.dependencies),
            "dev_dependencies": dependency_entries(&config.dev_dependencies),
            "build_dependencies": dependency_entries(&config.build_dependencies),
        },
        "targets": config.target_dependencies
            .iter()
            .map(target_dependency_json)
            .collect::<Vec<_>>(),
    })
}

fn target_matrix_status(config: &KoboProjectConfig) -> &'static str {
    if config.target_dependencies.is_empty() {
        "host_only"
    } else {
        "preserved"
    }
}

fn target_dependency_json(target: &TargetDependencyConfig) -> serde_json::Value {
    serde_json::json!({
        "target": target.target,
        "dependencies": dependency_entries(&target.dependencies),
        "dev_dependencies": dependency_entries(&target.dev_dependencies),
        "build_dependencies": dependency_entries(&target.build_dependencies),
        "release_workflows": ["cargo_check", "cargo_clippy", "cargo_test"],
    })
}

fn dependency_entries(dependencies: &[(String, String)]) -> Vec<serde_json::Value> {
    dependencies
        .iter()
        .map(|(name, version)| {
            serde_json::json!({
                "name": name,
                "version": version,
            })
        })
        .collect()
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

fn source_map_relative_path(kobo_path: &Path, source_count: usize) -> PathBuf {
    if source_count == 1 {
        return PathBuf::from("main.kobo.map");
    }
    let mut rel = kobo_path
        .strip_prefix("src")
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| kobo_path.to_path_buf());
    rel.set_extension("kobo.map");
    rel
}

fn normalized_display(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn generate_cargo_toml(config: &KoboProjectConfig) -> String {
    let mut toml = String::new();
    toml.push_str("[package]\n");
    toml.push_str(&format!("name = \"{}\"\n", config.name));
    toml.push_str(&format!("version = \"{}\"\n", config.version));
    toml.push_str(&format!("edition = \"{}\"\n", config.edition));
    toml.push('\n');

    if !config.dependencies.is_empty() {
        push_dependency_section(&mut toml, "dependencies", &config.dependencies);
    }
    if !config.build_dependencies.is_empty() {
        push_dependency_section(&mut toml, "build-dependencies", &config.build_dependencies);
    }
    if !config.dev_dependencies.is_empty() {
        push_dependency_section(&mut toml, "dev-dependencies", &config.dev_dependencies);
    }
    for target in &config.target_dependencies {
        push_dependency_section_if_not_empty(
            &mut toml,
            &target.target,
            "dependencies",
            &target.dependencies,
        );
        push_dependency_section_if_not_empty(
            &mut toml,
            &target.target,
            "build-dependencies",
            &target.build_dependencies,
        );
        push_dependency_section_if_not_empty(
            &mut toml,
            &target.target,
            "dev-dependencies",
            &target.dev_dependencies,
        );
    }

    toml.push_str("[workspace]\n");

    toml
}

fn push_dependency_section_if_not_empty(
    toml: &mut String,
    target: &str,
    section: &str,
    dependencies: &[(String, String)],
) {
    if dependencies.is_empty() {
        return;
    }
    push_dependency_section(
        toml,
        &format!("target.\"{}\".{section}", target.replace('"', "\\\"")),
        dependencies,
    );
}

fn push_dependency_section(toml: &mut String, section: &str, dependencies: &[(String, String)]) {
    toml.push_str(&format!("[{section}]\n"));
    for (dep_name, dep_version) in dependencies {
        if dep_version.trim_start().starts_with('{') {
            toml.push_str(&format!("{dep_name} = {dep_version}\n"));
        } else {
            toml.push_str(&format!("{dep_name} = \"{dep_version}\"\n"));
        }
    }
    toml.push('\n');
}

fn config_with_inferred_dependencies(
    config: &KoboProjectConfig,
    source_files: &[CargoSourceFile],
) -> KoboProjectConfig {
    let mut config = config.clone();
    let needs_tokio = source_files
        .iter()
        .any(|source_file| source_file.clean_source.contains("tokio::"));
    let has_tokio = config.dependencies.iter().any(|(name, _)| name == "tokio");
    let needs_rayon = source_files.iter().any(|source_file| {
        source_file.clean_source.contains("rayon::")
            || source_file.clean_source.contains(".par_iter()")
    });
    let has_rayon = config.dependencies.iter().any(|(name, _)| name == "rayon");

    if needs_tokio && !has_tokio {
        config.dependencies.push((
            "tokio".to_owned(),
            "{ version = \"1\", features = [\"rt-multi-thread\", \"macros\", \"sync\", \"time\"] }"
                .to_owned(),
        ));
    }
    if needs_rayon && !has_rayon {
        config
            .dependencies
            .push(("rayon".to_owned(), "1".to_owned()));
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
    fn cargo_generation_infers_rayon_dependency_from_generated_source() {
        let config = KoboProjectConfig::default();
        let sources = vec![(
            PathBuf::from("src/main.kobo"),
            "use rayon::prelude::*;\nfn main() { let values = vec![1]; values.par_iter().for_each(|_| {}); }".to_owned(),
        )];

        let temp = tempfile::tempdir().unwrap();
        generate_cargo_project(&config, &sources, temp.path()).unwrap();

        let cargo_toml = fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("rayon = \"1\""));
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
            ..Default::default()
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
