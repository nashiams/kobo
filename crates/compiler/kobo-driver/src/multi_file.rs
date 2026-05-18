use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::KoboConfig;
use crate::errors::DriverError;
use crate::session::CompileSession;
use kobo_ir::MustCallObligation;
use syn::visit::Visit;

/// Output produced by the multi-file build pipeline.
pub struct BuildOutput {
    pub gen_dir: PathBuf,
    pub rs_files: Vec<PathBuf>,
    pub cargo_toml_path: PathBuf,
    pub binary_path: Option<PathBuf>,
    pub diagnostics: Vec<String>,
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
pub fn generate_cargo_toml(
    config: &KoboConfig,
    project_dir: &Path,
    gen_dir: &Path,
) -> std::io::Result<()> {
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
    if let Some(build_script) = package_build_script(project_dir) {
        cargo.push_str(&format!("build = \"{build_script}\"\n"));
    }

    cargo.push_str("\n[workspace]\n");

    if !config.workspace_dependencies.is_empty() {
        cargo.push_str("\n[workspace.dependencies]\n");
        push_dependency_entries(
            &mut cargo,
            project_dir,
            gen_dir,
            &config.workspace_dependencies,
        );
    }

    if !config.dependencies.is_empty() {
        cargo.push_str("\n[dependencies]\n");
        push_dependency_entries(&mut cargo, project_dir, gen_dir, &config.dependencies);
    }

    if !config.build_dependencies.is_empty() {
        cargo.push_str("\n[build-dependencies]\n");
        push_dependency_entries(&mut cargo, project_dir, gen_dir, &config.build_dependencies);
    }

    if !config.dev_dependencies.is_empty() {
        cargo.push_str("\n[dev-dependencies]\n");
        push_dependency_entries(&mut cargo, project_dir, gen_dir, &config.dev_dependencies);
    }

    for target in &config.target_dependencies {
        push_target_dependency_section(
            &mut cargo,
            project_dir,
            gen_dir,
            &target.target,
            "dependencies",
            &target.dependencies,
        );
        push_target_dependency_section(
            &mut cargo,
            project_dir,
            gen_dir,
            &target.target,
            "build-dependencies",
            &target.build_dependencies,
        );
        push_target_dependency_section(
            &mut cargo,
            project_dir,
            gen_dir,
            &target.target,
            "dev-dependencies",
            &target.dev_dependencies,
        );
    }

    fs::create_dir_all(gen_dir)?;
    fs::write(gen_dir.join("Cargo.toml"), cargo)
}

fn push_target_dependency_section(
    cargo: &mut String,
    project_dir: &Path,
    gen_dir: &Path,
    target: &str,
    section: &str,
    dependencies: &std::collections::HashMap<String, toml::Value>,
) {
    if dependencies.is_empty() {
        return;
    }
    cargo.push_str(&format!(
        "\n[target.\"{}\".{section}]\n",
        target.replace('"', "\\\"")
    ));
    push_dependency_entries(cargo, project_dir, gen_dir, dependencies);
}

fn push_dependency_entries(
    cargo: &mut String,
    project_dir: &Path,
    gen_dir: &Path,
    dependencies: &std::collections::HashMap<String, toml::Value>,
) {
    let mut entries = dependencies.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (dep_name, dep_value) in entries {
        let generated_value =
            dependency_value_for_generated_project(project_dir, gen_dir, dep_value);
        match &generated_value {
            toml::Value::String(ver) => {
                cargo.push_str(&format!("{dep_name} = \"{ver}\"\n"));
            }
            other => {
                cargo.push_str(&format!("{dep_name} = {other}\n"));
            }
        }
    }
}

fn dependency_value_for_generated_project(
    project_dir: &Path,
    gen_dir: &Path,
    value: &toml::Value,
) -> toml::Value {
    let toml::Value::Table(table) = value else {
        return value.clone();
    };
    let mut table = table.clone();
    if let Some(path_value) = table
        .get("path")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
    {
        let source_path = if Path::new(&path_value).is_absolute() {
            PathBuf::from(&path_value)
        } else {
            project_dir.join(&path_value)
        };
        table.insert(
            "path".to_owned(),
            toml::Value::String(path_from_generated_project(gen_dir, &source_path)),
        );
    }
    toml::Value::Table(table)
}

fn path_from_generated_project(gen_dir: &Path, source_path: &Path) -> String {
    if let Some(project_dir) = gen_dir.parent().and_then(Path::parent) {
        if let Ok(relative_to_project) = source_path.strip_prefix(project_dir) {
            return normalize_manifest_path(PathBuf::from("../..").join(relative_to_project));
        }
    }
    normalize_manifest_path(source_path.to_path_buf())
}

fn normalize_manifest_path(path: PathBuf) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Runs the build pipeline: discovers .kobo files, compiles each to .rs,
/// generates Cargo.toml, shells out to `cargo build`, and writes everything
/// under `<project_dir>/target/kobo-gen/`.
pub fn run_build_pipeline(
    config: &KoboConfig,
    project_dir: &Path,
) -> Result<BuildOutput, DriverError> {
    let gen_dir = project_dir.join("target").join("kobo-gen");
    let gen_src_dir = gen_dir.join("src");
    fs::create_dir_all(&gen_src_dir)?;

    let kobo_files = discover_kobo_files(project_dir, config);
    if kobo_files.is_empty() {
        return Err(DriverError::ParseFailed {
            file: project_dir.join(&config.src_dir),
            message: "no .kobo files found".to_string(),
        });
    }

    let src_dir = project_dir.join(&config.src_dir);
    let mut rs_files = Vec::new();
    let mut summary_obligations = Vec::new();
    let mut summary_functions = Vec::new();
    let mut summary_source = String::new();

    for kobo_path in &kobo_files {
        let source = fs::read_to_string(kobo_path)?;
        summary_source.push_str(&source);
        summary_functions.extend(summary_functions_json(&source, config));
        let rel = kobo_path
            .strip_prefix(&src_dir)
            .map_err(|e| DriverError::TransformFailed {
                file: kobo_path.clone(),
                message: format!("path error: {e}"),
            })?;
        let rs_rel = rel.with_extension("rs");
        let rs_out = gen_src_dir.join(&rs_rel);

        if let Some(parent) = rs_out.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut build_config = config.clone();
        build_config.output_dir = Some(rs_out.parent().unwrap_or(&gen_src_dir).to_path_buf());

        let mut session = CompileSession::new(build_config);
        let artifacts =
            crate::pipeline::run_codegen_pipeline(&mut session, kobo_path).map_err(|()| {
                DriverError::TransformFailed {
                    file: kobo_path.clone(),
                    message: "compilation failed".to_string(),
                }
            })?;

        fs::write(&rs_out, &artifacts.rs_source)?;
        summary_obligations.extend(
            artifacts
                .must_call_obligations
                .iter()
                .map(|obligation| summary_obligation_json(obligation, &source, config)),
        );
        rs_files.push(rs_out);
    }

    copy_build_script_if_present(project_dir, &gen_dir)?;

    generate_cargo_toml(config, project_dir, &gen_dir).map_err(|e| {
        DriverError::CargoTomlGenFailed {
            message: e.to_string(),
        }
    })?;

    let cargo_toml_path = gen_dir.join("Cargo.toml");
    write_kobo_summary(
        config,
        &gen_dir,
        stable_hash(&summary_source),
        summary_functions,
        summary_obligations,
    )?;

    // Shell out to `cargo build`.
    let cargo_output = Command::new("cargo")
        .args(["build", "--message-format=json"])
        .current_dir(&gen_dir)
        .output()
        .map_err(DriverError::IoError)?;

    let mut diagnostics = Vec::new();
    let mut binary_path: Option<PathBuf> = None;

    // Parse JSON message stream from cargo.
    let stdout = String::from_utf8_lossy(&cargo_output.stdout);
    for line in stdout.lines() {
        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(reason) = msg.get("reason").and_then(|r| r.as_str()) {
                if reason == "compiler-artifact" {
                    if let Some(exe) = msg.get("executable").and_then(|e| e.as_str()) {
                        binary_path = Some(PathBuf::from(exe));
                    }
                }
                if reason == "compiler-message" {
                    if let Some(rendered) = msg
                        .get("message")
                        .and_then(|m| m.get("rendered"))
                        .and_then(|r| r.as_str())
                    {
                        diagnostics.push(rendered.to_string());
                    }
                }
            }
        }
    }

    if !cargo_output.status.success() {
        let stderr = String::from_utf8_lossy(&cargo_output.stderr).to_string();
        let exit_code = cargo_output.status.code().unwrap_or(1);
        return Err(DriverError::CargoBuildFailed { stderr, exit_code });
    }

    Ok(BuildOutput {
        gen_dir,
        rs_files,
        cargo_toml_path,
        binary_path,
        diagnostics,
    })
}

fn summary_obligation_json(
    obligation: &MustCallObligation,
    source: &str,
    config: &KoboConfig,
) -> serde_json::Value {
    let applicable_types = vec![qualify_type_path(config, &obligation.owner_type)];
    let applicable_functions = obligation_applicable_functions(source, config, obligation);
    serde_json::json!({
        "type": obligation.owner_type,
        "applicable_types": applicable_types.clone(),
        "applicable_functions": applicable_functions.clone(),
        "applicability": {
            "types": applicable_types,
            "functions": applicable_functions,
            "source": "producer-summary",
        },
        "applicability_source": "producer-summary",
        "terminal_actions": obligation
            .actions
            .iter()
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>(),
    })
}

fn qualify_type_path(config: &KoboConfig, type_name: &str) -> String {
    if type_name.contains("::") {
        return type_name.to_owned();
    }
    format!("{}::{type_name}", package_name(config))
}

fn obligation_applicable_functions(
    source: &str,
    config: &KoboConfig,
    obligation: &MustCallObligation,
) -> Vec<String> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let package = package_name(config);
    file.items
        .iter()
        .filter_map(|item| {
            let syn::Item::Fn(function) = item else {
                return None;
            };
            function_references_type(function, &obligation.owner_type)
                .then(|| format!("{package}::{}", function.sig.ident))
        })
        .collect()
}

fn function_references_type(function: &syn::ItemFn, type_name: &str) -> bool {
    let expected = split_path(type_name);
    if expected.is_empty() {
        return false;
    }
    let mut visitor = TypeReferenceVisitor {
        expected,
        found: false,
    };
    visitor.visit_signature(&function.sig);
    visitor.visit_block(&function.block);
    visitor.found
}

struct TypeReferenceVisitor {
    expected: Vec<String>,
    found: bool,
}

impl<'ast> Visit<'ast> for TypeReferenceVisitor {
    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        self.record_path(&node.path);
        syn::visit::visit_expr_struct(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        self.record_path(&node.path);
        syn::visit::visit_type_path(self, node);
    }
}

impl TypeReferenceVisitor {
    fn record_path(&mut self, path: &syn::Path) {
        let segments = path_segments(path);
        if segments == self.expected
            || (self.expected.len() == 1 && segments.last() == self.expected.first())
        {
            self.found = true;
        }
    }
}

fn split_path(path: &str) -> Vec<String> {
    path.split("::")
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn summary_functions_json(source: &str, config: &KoboConfig) -> Vec<serde_json::Value> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    file.items
        .iter()
        .filter_map(|item| {
            let syn::Item::Fn(function) = item else {
                return None;
            };
            let name = function.sig.ident.to_string();
            Some(serde_json::json!({
                "path": format!("{}::{name}", package_name(config)),
                "creates": [],
                "transfers": [],
                "discharges": [],
                "returns": [],
                "effects": effect_tags_for_function(function),
                "boundaries": boundary_tags_for_function(function),
            }))
        })
        .collect()
}

fn package_name(config: &KoboConfig) -> &str {
    if config.package_name.is_empty() {
        "kobo_project"
    } else {
        &config.package_name
    }
}

fn effect_tags_for_function(function: &syn::ItemFn) -> Vec<&'static str> {
    let body = quote::quote!(#function).to_string();
    let mut effects = Vec::new();
    if body.contains("std :: fs") {
        effects.push("filesystem");
    }
    if body.contains("std :: env") {
        effects.push("environment");
    }
    if body.contains("std :: process") {
        effects.push("process");
    }
    effects
}

fn boundary_tags_for_function(function: &syn::ItemFn) -> Vec<String> {
    function
        .attrs
        .iter()
        .filter(|attr| {
            attr.path()
                .segments
                .iter()
                .any(|segment| segment.ident == "boundary")
        })
        .map(|_| "source-boundary".to_owned())
        .collect()
}

fn write_kobo_summary(
    config: &KoboConfig,
    gen_dir: &Path,
    source_hash: String,
    functions: Vec<serde_json::Value>,
    obligations: Vec<serde_json::Value>,
) -> std::io::Result<()> {
    let summary_body = serde_json::json!({
        "schema_version": 1,
        "crate": if config.package_name.is_empty() { "kobo_project" } else { &config.package_name },
        "crate_version": if config.package_version.is_empty() { "0.1.0" } else { &config.package_version },
        "source_hash": source_hash,
        "functions": functions,
        "obligations": obligations,
    });
    let summary_hash = stable_hash(&summary_body.to_string());
    let mut summary = summary_body;
    summary["summary_hash"] = serde_json::Value::String(summary_hash);
    fs::write(
        gen_dir.join(".kobo-summary"),
        serde_json::to_string_pretty(&summary).expect(".kobo-summary should serialize"),
    )
}

fn stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn package_build_script(project_dir: &Path) -> Option<String> {
    let manifest = fs::read_to_string(project_dir.join("Cargo.toml")).ok()?;
    let parsed = manifest.parse::<toml::Value>().ok()?;
    parsed
        .get("package")?
        .get("build")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            project_dir
                .join("build.rs")
                .is_file()
                .then(|| "build.rs".to_owned())
        })
}

fn copy_build_script_if_present(project_dir: &Path, gen_dir: &Path) -> std::io::Result<()> {
    let Some(build_script) = package_build_script(project_dir) else {
        return Ok(());
    };
    let source = project_dir.join(&build_script);
    if source.is_file() {
        fs::create_dir_all(gen_dir)?;
        fs::copy(source, gen_dir.join(build_script))?;
    }
    Ok(())
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

        let mut config = KoboConfig {
            package_name: "test_pkg".to_string(),
            package_version: "1.0.0".to_string(),
            ..Default::default()
        };
        config
            .dependencies
            .insert("log".to_string(), toml::Value::String("0.4".to_string()));

        generate_cargo_toml(&config, &tmp, &tmp).unwrap();

        let content = fs::read_to_string(tmp.join("Cargo.toml")).unwrap();
        assert!(content.contains("name = \"test_pkg\""));
        assert!(content.contains("version = \"1.0.0\""));
        assert!(content.contains("log = \"0.4\""));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn generate_cargo_toml_preserves_non_regular_dependency_sections() {
        let tmp = std::env::temp_dir().join(format!("kobo-cargo-sections-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let gen_dir = tmp.join("target").join("kobo-gen");

        let mut config = KoboConfig {
            package_name: "section_pkg".to_string(),
            package_version: "1.0.0".to_string(),
            ..Default::default()
        };
        config.build_dependencies.insert(
            "build_helper".to_string(),
            toml::Value::Table(toml::Table::from_iter([(
                "path".to_string(),
                toml::Value::String("build_helper".to_string()),
            )])),
        );
        config.dev_dependencies.insert(
            "dev_helper".to_string(),
            toml::Value::Table(toml::Table::from_iter([(
                "path".to_string(),
                toml::Value::String("dev_helper".to_string()),
            )])),
        );
        config
            .target_dependencies
            .push(crate::config::CargoTargetDependencyConfig {
                target: "cfg(windows)".to_string(),
                dependencies: std::collections::HashMap::from([(
                    "target_helper".to_string(),
                    toml::Value::Table(toml::Table::from_iter([(
                        "path".to_string(),
                        toml::Value::String("target_helper".to_string()),
                    )])),
                )]),
                dev_dependencies: std::collections::HashMap::from([(
                    "target_dev_helper".to_string(),
                    toml::Value::Table(toml::Table::from_iter([(
                        "path".to_string(),
                        toml::Value::String("target_dev_helper".to_string()),
                    )])),
                )]),
                build_dependencies: std::collections::HashMap::from([(
                    "target_build_helper".to_string(),
                    toml::Value::Table(toml::Table::from_iter([(
                        "path".to_string(),
                        toml::Value::String("target_build_helper".to_string()),
                    )])),
                )]),
            });

        generate_cargo_toml(&config, &tmp, &gen_dir).unwrap();

        let content = fs::read_to_string(gen_dir.join("Cargo.toml")).unwrap();
        for expected in [
            "[build-dependencies]",
            r#"build_helper = { path = "../../build_helper" }"#,
            "[dev-dependencies]",
            r#"dev_helper = { path = "../../dev_helper" }"#,
            "[target.\"cfg(windows)\".dependencies]",
            r#"target_helper = { path = "../../target_helper" }"#,
            "[target.\"cfg(windows)\".build-dependencies]",
            r#"target_build_helper = { path = "../../target_build_helper" }"#,
            "[target.\"cfg(windows)\".dev-dependencies]",
            r#"target_dev_helper = { path = "../../target_dev_helper" }"#,
        ] {
            assert!(
                content.contains(expected),
                "missing {expected} in generated manifest:\n{content}"
            );
        }

        let _ = fs::remove_dir_all(&tmp);
    }
}
