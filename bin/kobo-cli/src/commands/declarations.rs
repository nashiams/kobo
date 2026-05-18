use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::ecosystem::{sha256_hex, version_requirement_allows};

#[derive(Clone)]
pub(super) struct DeclarationFacts {
    pub(super) path: PathBuf,
    pub(super) version: String,
    pub(super) schema_version: i64,
    pub(super) hash: String,
    pub(super) metadata_package: Option<DeclarationMetadataPackage>,
    pub(super) types: Vec<String>,
    pub(super) type_facts: Vec<DeclarationTypeFact>,
    pub(super) functions: Vec<String>,
    pub(super) function_facts: Vec<DeclarationFunctionFact>,
    pub(super) effects: Vec<String>,
    pub(super) adapters: Vec<String>,
    pub(super) activity_facts: Vec<DeclarationActivityFact>,
}

#[derive(Clone)]
pub(super) struct DeclarationTypeFact {
    pub(super) path: String,
    pub(super) kind: Option<String>,
    pub(super) resource: bool,
    pub(super) must_call: Vec<String>,
    pub(super) ownership: Option<String>,
    pub(super) strict_ward: Option<String>,
}

#[derive(Clone)]
pub(super) struct DeclarationMetadataPackage {
    pub(super) package: String,
    pub(super) version: Option<String>,
    pub(super) path: PathBuf,
    pub(super) source: Option<String>,
    pub(super) registry: Option<String>,
    pub(super) checksum: Option<String>,
    pub(super) signed_by: Option<String>,
    pub(super) validated: bool,
}

#[derive(Clone)]
pub(super) struct DeclarationFunctionFact {
    pub(super) path: String,
    pub(super) effects: Vec<String>,
    pub(super) simulation: Option<String>,
    pub(super) determinism: Option<String>,
    pub(super) returns: Option<String>,
    pub(super) obligation: Option<String>,
    pub(super) strict_ward: Option<String>,
}

#[derive(Clone)]
pub(super) struct DeclarationActivityFact {
    pub(super) path: String,
    pub(super) retry: Option<String>,
    pub(super) idempotency: Option<String>,
    pub(super) result: Option<String>,
    pub(super) compensation: Option<String>,
}

pub(super) struct DeclarationError {
    pub(super) path: PathBuf,
    pub(super) key: &'static str,
    pub(super) message: String,
}

pub(super) enum DeclarationLookup {
    Valid(DeclarationFacts),
    Missing,
    Invalid(DeclarationError),
}

pub(super) fn load_declaration(
    file: &Path,
    crate_name: &str,
    expected_version: Option<&str>,
) -> DeclarationLookup {
    let Some(search_root) = file.parent() else {
        return DeclarationLookup::Missing;
    };
    for ancestor in search_root.ancestors() {
        for candidate in declaration_candidates(ancestor, crate_name) {
            if candidate.is_file() {
                return parse_declaration_file(&candidate, crate_name, expected_version);
            }
        }
    }
    DeclarationLookup::Missing
}

pub(super) fn load_declaration_with_config(
    file: &Path,
    crate_name: &str,
    expected_version: Option<&str>,
    config: &kobo_driver::KoboConfig,
) -> DeclarationLookup {
    match load_declaration(file, crate_name, expected_version) {
        DeclarationLookup::Missing => {}
        other => return other,
    }
    let Some(types_package) = config.ecosystem_policy.types_for(crate_name) else {
        return DeclarationLookup::Missing;
    };
    load_declaration_from_types_package(crate_name, expected_version, types_package)
}

pub(super) fn declaration_facts_for_config(
    file: &Path,
    crate_name: &str,
    config: &kobo_driver::KoboConfig,
) -> DeclarationLookup {
    let expected_version = dependency_version(config, crate_name);
    load_declaration_with_config(file, crate_name, expected_version.as_deref(), config)
}

pub(super) fn declaration_facts_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    crate_name: &str,
    policy: &str,
    call_path: Option<&str>,
) -> Result<Option<DeclarationFacts>, DeclarationError> {
    if !matches!(policy, "typed" | "activity") {
        return Ok(None);
    }
    let facts = match declaration_facts_for_config(file, crate_name, config) {
        DeclarationLookup::Valid(facts) => facts,
        DeclarationLookup::Missing => {
            return Err(DeclarationError {
                path: file.to_path_buf(),
                key: if policy == "activity" {
                    "activity"
                } else {
                    "declaration"
                },
                message: format!(
                    "`{crate_name}` is marked {policy}, but Kobo could not find a matching kobo.d.toml declaration"
                ),
            });
        }
        DeclarationLookup::Invalid(error) => return Err(error),
    };
    if policy == "typed" {
        if !declaration_covers_call(&facts, call_path) {
            return Err(DeclarationError {
                path: facts.path.clone(),
                key: "function",
                message: format!(
                    "declaration does not cover boundary call `{}`",
                    call_path.unwrap_or("<unknown external call>")
                ),
            });
        }
    }
    if policy == "activity" && !activity_covers_call(Some(&facts), call_path) {
        return Err(DeclarationError {
            path: facts.path.clone(),
            key: "activity",
            message: format!(
                "activity declaration for `{crate_name}` does not declare complete retry, idempotency, result, and compensation metadata for `{}`",
                call_path.unwrap_or("<unknown external call>")
            ),
        });
    }
    Ok(Some(facts))
}

pub(super) fn declaration_candidates(root: &Path, crate_name: &str) -> Vec<PathBuf> {
    vec![
        root.join(format!("{crate_name}.kobo.d.toml")),
        root.join("crate.kobo.d.toml"),
        root.join("kobo.d.toml"),
        root.join("kobo.d").join(format!("{crate_name}.toml")),
        root.join("declarations")
            .join(format!("{crate_name}.kobo.d.toml")),
        root.join(".kobo")
            .join("declarations")
            .join(format!("{crate_name}.kobo.d.toml")),
    ]
}

pub(super) fn dependency_version(
    config: &kobo_driver::KoboConfig,
    crate_name: &str,
) -> Option<String> {
    for dependencies in [
        &config.dependencies,
        &config.dev_dependencies,
        &config.build_dependencies,
        &config.workspace_dependencies,
    ] {
        if let Some(version) = dependency_version_in(dependencies, crate_name) {
            return Some(version);
        }
    }
    for target in &config.target_dependencies {
        for dependencies in [
            &target.dependencies,
            &target.dev_dependencies,
            &target.build_dependencies,
        ] {
            if let Some(version) = dependency_version_in(dependencies, crate_name) {
                return Some(version);
            }
        }
    }
    None
}

fn dependency_version_in(
    dependencies: &HashMap<String, toml::Value>,
    crate_name: &str,
) -> Option<String> {
    dependencies.iter().find_map(|(alias, value)| {
        if alias == crate_name || dependency_package(value).as_deref() == Some(crate_name) {
            return dependency_value_version(value);
        }
        None
    })
}

fn dependency_package(value: &toml::Value) -> Option<String> {
    value
        .as_table()
        .and_then(|table| table.get("package"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}

fn dependency_value_version(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(version) => Some(version.clone()),
        toml::Value::Table(table) => table
            .get("version")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

pub(super) fn parse_declaration_file(
    path: &Path,
    crate_name: &str,
    expected_version: Option<&str>,
) -> DeclarationLookup {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "file",
                message: error.to_string(),
            })
        }
    };
    let parsed = match source.parse::<toml::Value>() {
        Ok(parsed) => parsed,
        Err(error) => {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "crate.name",
                message: error.to_string(),
            })
        }
    };
    let schema_version = parsed
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if schema_version != 0 {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "schema_version",
            message: "only declaration schema_version 0 is supported".to_owned(),
        });
    }
    let Some(crate_table) = parsed.get("crate").and_then(toml::Value::as_table) else {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "crate",
            message: "missing [crate] table".to_owned(),
        });
    };
    if crate_table.get("name").and_then(toml::Value::as_str) != Some(crate_name) {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "crate.name",
            message: format!("expected declaration for crate `{crate_name}`"),
        });
    }
    let Some(version) = crate_table.get("version").and_then(toml::Value::as_str) else {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "crate.version",
            message: "missing declaration crate version".to_owned(),
        });
    };
    if let Some(expected_version) = expected_version {
        if version != expected_version {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "crate.version",
                message: format!("expected version `{expected_version}`, found `{version}`"),
            });
        }
    }
    let observed_declaration_hash = declaration_hash(&source);
    if let Some(declared_hash) = crate_table
        .get("declaration_hash")
        .and_then(toml::Value::as_str)
    {
        if declared_hash != observed_declaration_hash {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "crate.declaration_hash",
                message: format!(
                    "expected declaration_hash `{declared_hash}`, found `{observed_declaration_hash}`"
                ),
            });
        }
    }

    DeclarationLookup::Valid(DeclarationFacts {
        path: path.to_path_buf(),
        version: version.to_owned(),
        schema_version,
        hash: observed_declaration_hash,
        metadata_package: None,
        types: declaration_type_paths(&parsed),
        type_facts: declaration_type_facts(&parsed),
        functions: declaration_function_paths(&parsed),
        function_facts: declaration_function_facts(&parsed),
        effects: declaration_effects(&parsed),
        adapters: declaration_adapter_paths(&parsed),
        activity_facts: declaration_activity_facts(&parsed),
    })
}

fn load_declaration_from_types_package(
    crate_name: &str,
    expected_version: Option<&str>,
    types_package: &kobo_driver::EcosystemTypesPolicy,
) -> DeclarationLookup {
    let Some(metadata_path) = types_package.metadata_path.as_deref() else {
        if let Some(facts) = builtin_declaration_facts(crate_name, expected_version, types_package)
        {
            return DeclarationLookup::Valid(facts);
        }
        return DeclarationLookup::Missing;
    };
    let package = match parse_types_package(metadata_path, types_package, expected_version) {
        Ok(package) => package,
        Err(error) => return error,
    };
    let lookup = parse_declaration_file(&package.declaration_path, crate_name, expected_version);
    match lookup {
        DeclarationLookup::Valid(mut facts) => {
            if let Some(expected_hash) = package.declaration_hash.as_deref() {
                if facts.hash != expected_hash {
                    return DeclarationLookup::Invalid(DeclarationError {
                        path: package.declaration_path,
                        key: "declaration_hash",
                        message: format!(
                            "metadata package expected declaration_hash `{expected_hash}`, found `{}`",
                            facts.hash
                        ),
                    });
                }
            }
            facts.metadata_package = Some(DeclarationMetadataPackage {
                package: types_package.package.clone(),
                version: types_package.version.clone(),
                path: metadata_path.to_path_buf(),
                source: types_package.source.clone(),
                registry: types_package.registry.clone(),
                checksum: types_package.checksum.clone(),
                signed_by: types_package.signed_by.clone(),
                validated: types_package.validated,
            });
            DeclarationLookup::Valid(facts)
        }
        other => other,
    }
}

fn builtin_declaration_facts(
    crate_name: &str,
    expected_version: Option<&str>,
    types_package: &kobo_driver::EcosystemTypesPolicy,
) -> Option<DeclarationFacts> {
    if types_package.source.as_deref() != Some("registry")
        || types_package.registry.as_deref() != Some(super::ecosystem::BUILTIN_REGISTRY_NAME)
        || !types_package.validated
    {
        return None;
    }
    let version = builtin_declaration_version(crate_name)?;
    if let Some(expected_version) = expected_version {
        if expected_version != version {
            return None;
        }
    }
    let (type_facts, function_facts, activity_facts) = builtin_declaration_catalog(crate_name)?;
    let types = type_facts
        .iter()
        .map(|fact| fact.path.clone())
        .collect::<Vec<_>>();
    let functions = function_facts
        .iter()
        .map(|fact| fact.path.clone())
        .collect::<Vec<_>>();
    let hash_material = format!(
        "{}:{}:{}:{}",
        crate_name,
        version,
        types.join(","),
        functions.join(",")
    );
    Some(DeclarationFacts {
        path: PathBuf::from(format!("<builtin:{}>", types_package.package)),
        version: version.to_owned(),
        schema_version: 0,
        hash: stable_hash(&hash_material),
        metadata_package: Some(DeclarationMetadataPackage {
            package: types_package.package.clone(),
            version: types_package.version.clone(),
            path: PathBuf::from(format!("<builtin:{}>", types_package.package)),
            source: types_package.source.clone(),
            registry: types_package.registry.clone(),
            checksum: types_package.checksum.clone(),
            signed_by: types_package.signed_by.clone(),
            validated: types_package.validated,
        }),
        types,
        type_facts,
        functions,
        function_facts,
        effects: Vec::new(),
        adapters: Vec::new(),
        activity_facts,
    })
}

fn builtin_declaration_version(crate_name: &str) -> Option<&'static str> {
    match crate_name {
        "serde_json" => Some("1"),
        "reqwest" => Some("0.12"),
        "sqlx" => Some("0.8"),
        "tokio" => Some("1"),
        "anyhow" => Some("1"),
        "clap" => Some("4"),
        "thiserror" => Some("1"),
        _ => None,
    }
}

fn builtin_declaration_catalog(
    crate_name: &str,
) -> Option<(
    Vec<DeclarationTypeFact>,
    Vec<DeclarationFunctionFact>,
    Vec<DeclarationActivityFact>,
)> {
    let catalog = match crate_name {
        "serde_json" => (
            vec![
                builtin_type("serde_json::Value", "enum"),
                builtin_type("serde_json::Map", "struct"),
                builtin_type("serde_json::Number", "struct"),
            ],
            vec![
                builtin_function("serde_json::from_str", Some("serde_json::Value")),
                builtin_function("serde_json::to_string", Some("String")),
                builtin_function("serde_json::to_vec", Some("Vec")),
            ],
            Vec::new(),
        ),
        "reqwest" => (
            vec![
                builtin_type("reqwest::Client", "struct"),
                builtin_type("reqwest::RequestBuilder", "struct"),
                builtin_type("reqwest::Response", "struct"),
                builtin_type("reqwest::Error", "struct"),
            ],
            vec![
                builtin_function("reqwest::Client::new", Some("reqwest::Client")),
                builtin_function("reqwest::Client::get", Some("reqwest::RequestBuilder")),
                builtin_function("reqwest::get", Some("reqwest::Response")),
            ],
            vec![
                builtin_activity("reqwest::Client::get"),
                builtin_activity("reqwest::get"),
            ],
        ),
        "sqlx" => (
            vec![
                builtin_type("sqlx::Pool", "struct"),
                builtin_type("sqlx::Transaction", "struct"),
                builtin_type("sqlx::Executor", "trait"),
                builtin_type("sqlx::Row", "trait"),
            ],
            vec![
                builtin_function("sqlx::Pool::connect_lazy", Some("sqlx::Pool")),
                builtin_function("sqlx::query", None),
                builtin_function("sqlx::query_as", None),
            ],
            vec![
                builtin_activity("sqlx::query"),
                builtin_activity("sqlx::query_as"),
            ],
        ),
        "tokio" => (
            vec![
                builtin_type("tokio::task::JoinHandle", "struct"),
                builtin_type("tokio::runtime::Runtime", "struct"),
            ],
            vec![
                builtin_function("tokio::spawn", Some("tokio::task::JoinHandle")),
                builtin_function("tokio::task::yield_now", None),
                builtin_function("tokio::time::sleep", None),
            ],
            Vec::new(),
        ),
        "anyhow" => (
            vec![
                builtin_type("anyhow::Error", "struct"),
                builtin_type("anyhow::Result", "type_alias"),
            ],
            vec![builtin_function("anyhow::anyhow", Some("anyhow::Error"))],
            Vec::new(),
        ),
        "clap" => (
            vec![
                builtin_type("clap::Command", "struct"),
                builtin_type("clap::Arg", "struct"),
            ],
            vec![builtin_function(
                "clap::Command::new",
                Some("clap::Command"),
            )],
            Vec::new(),
        ),
        "thiserror" => (
            vec![builtin_type("thiserror::Error", "trait")],
            Vec::new(),
            Vec::new(),
        ),
        _ => return None,
    };
    Some(catalog)
}

fn builtin_type(path: &str, kind: &str) -> DeclarationTypeFact {
    DeclarationTypeFact {
        path: path.to_owned(),
        kind: Some(kind.to_owned()),
        resource: false,
        must_call: Vec::new(),
        ownership: None,
        strict_ward: None,
    }
}

fn builtin_function(path: &str, returns: Option<&str>) -> DeclarationFunctionFact {
    DeclarationFunctionFact {
        path: path.to_owned(),
        effects: Vec::new(),
        simulation: Some("typed".to_owned()),
        determinism: Some("deterministic".to_owned()),
        returns: returns.map(str::to_owned),
        obligation: None,
        strict_ward: None,
    }
}

fn builtin_activity(path: &str) -> DeclarationActivityFact {
    DeclarationActivityFact {
        path: path.to_owned(),
        retry: Some("retry-reviewed".to_owned()),
        idempotency: Some("boundary-key".to_owned()),
        result: Some("record".to_owned()),
        compensation: Some("manual-review".to_owned()),
    }
}

struct TypesPackageDeclaration {
    declaration_path: PathBuf,
    declaration_hash: Option<String>,
}

fn parse_types_package(
    metadata_path: &Path,
    types_package: &kobo_driver::EcosystemTypesPolicy,
    expected_version: Option<&str>,
) -> Result<TypesPackageDeclaration, DeclarationLookup> {
    let source = std::fs::read_to_string(metadata_path).map_err(|error| {
        DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "metadata_path",
            message: error.to_string(),
        })
    })?;
    let parsed = source.parse::<toml::Value>().map_err(|error| {
        DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "metadata_path",
            message: error.to_string(),
        })
    })?;
    validate_configured_types_package(metadata_path, &source, types_package, expected_version)?;
    let schema_version = parsed
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if schema_version != 1 {
        return Err(DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "schema_version",
            message: "types metadata packages must use schema_version 1".to_owned(),
        }));
    }
    if parsed.get("kind").and_then(toml::Value::as_str) != Some("types") {
        return Err(DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "kind",
            message: "metadata package kind must be `types`".to_owned(),
        }));
    }
    if parsed.get("package").and_then(toml::Value::as_str) != Some(types_package.package.as_str()) {
        return Err(DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "package",
            message: format!("expected metadata package `{}`", types_package.package),
        }));
    }
    let declaration_path = parsed
        .get("declaration_path")
        .and_then(toml::Value::as_str)
        .or_else(|| {
            parsed
                .get("declaration")
                .and_then(toml::Value::as_table)
                .and_then(|table| table.get("path"))
                .and_then(toml::Value::as_str)
        })
        .ok_or_else(|| {
            DeclarationLookup::Invalid(DeclarationError {
                path: metadata_path.to_path_buf(),
                key: "declaration_path",
                message: "types metadata package must point at a declaration file".to_owned(),
            })
        })?;
    let declaration_hash = parsed
        .get("declaration_hash")
        .and_then(toml::Value::as_str)
        .or_else(|| {
            parsed
                .get("declaration")
                .and_then(toml::Value::as_table)
                .and_then(|table| table.get("hash"))
                .and_then(toml::Value::as_str)
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            DeclarationLookup::Invalid(DeclarationError {
                path: metadata_path.to_path_buf(),
                key: "declaration_hash",
                message: "types metadata package must pin the declaration_hash".to_owned(),
            })
        })?;
    let declaration_path = if Path::new(declaration_path).is_absolute() {
        PathBuf::from(declaration_path)
    } else {
        metadata_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(declaration_path)
    };
    Ok(TypesPackageDeclaration {
        declaration_path,
        declaration_hash: Some(declaration_hash),
    })
}

fn validate_configured_types_package(
    metadata_path: &Path,
    source: &str,
    types_package: &kobo_driver::EcosystemTypesPolicy,
    expected_version: Option<&str>,
) -> Result<(), DeclarationLookup> {
    let parsed = source.parse::<toml::Value>().map_err(|error| {
        DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "metadata_path",
            message: error.to_string(),
        })
    })?;
    if !types_package.validated {
        return Err(invalid_types_package(
            metadata_path,
            "validated",
            "configured metadata packages must be registry-validated before use",
        ));
    }
    if types_package.trust_policy.as_deref() != Some("workspace-pinned") {
        return Err(invalid_types_package(
            metadata_path,
            "trust_policy",
            "configured metadata packages must use trust_policy = \"workspace-pinned\"",
        ));
    }
    let Some(checksum) = types_package.checksum.as_deref() else {
        return Err(invalid_types_package(
            metadata_path,
            "checksum",
            "configured metadata packages must pin a sha256 checksum",
        ));
    };
    validate_configured_checksum(metadata_path, source, checksum)?;
    validate_configured_signer(metadata_path, &parsed, types_package)?;
    validate_configured_compatibility(metadata_path, &parsed, types_package, expected_version)?;
    Ok(())
}

fn validate_configured_checksum(
    metadata_path: &Path,
    source: &str,
    checksum: &str,
) -> Result<(), DeclarationLookup> {
    let Some(digest) = checksum.strip_prefix("sha256:") else {
        return Err(invalid_types_package(
            metadata_path,
            "checksum",
            "configured metadata package checksum must use sha256:<digest>",
        ));
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_types_package(
            metadata_path,
            "checksum",
            "configured metadata package checksum must use sha256:<64 hex digits>",
        ));
    }
    let actual = format!("sha256:{}", sha256_hex(source.as_bytes()));
    if actual != checksum {
        return Err(DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "checksum",
            message: format!(
                "configured metadata package checksum mismatch: expected {checksum}, found {actual}"
            ),
        }));
    }
    Ok(())
}

fn validate_configured_signer(
    metadata_path: &Path,
    parsed: &toml::Value,
    types_package: &kobo_driver::EcosystemTypesPolicy,
) -> Result<(), DeclarationLookup> {
    let configured_signer = types_package.signed_by.as_deref();
    let package_signer = parsed.get("signed_by").and_then(toml::Value::as_str);
    let Some(configured) = configured_signer else {
        return Err(invalid_types_package(
            metadata_path,
            "signed_by",
            "configured metadata packages must pin a signer",
        ));
    };
    let Some(package) = package_signer else {
        return Err(invalid_types_package(
            metadata_path,
            "signed_by",
            "metadata package must declare signed_by",
        ));
    };
    if configured != package {
        return Err(DeclarationLookup::Invalid(DeclarationError {
            path: metadata_path.to_path_buf(),
            key: "signed_by",
            message: format!("expected signer `{configured}`, found `{package}`"),
        }));
    }
    Ok(())
}

fn validate_configured_compatibility(
    metadata_path: &Path,
    parsed: &toml::Value,
    types_package: &kobo_driver::EcosystemTypesPolicy,
    expected_version: Option<&str>,
) -> Result<(), DeclarationLookup> {
    let package_compatible = parsed.get("compatible_crate").and_then(toml::Value::as_str);
    if let (Some(configured), Some(package)) = (
        types_package.compatible_crate.as_deref(),
        package_compatible,
    ) {
        if configured != package {
            return Err(DeclarationLookup::Invalid(DeclarationError {
                path: metadata_path.to_path_buf(),
                key: "compatible_crate",
                message: format!(
                    "configured compatibility `{configured}` does not match metadata package `{package}`"
                ),
            }));
        }
    }
    let Some(requirement) = types_package
        .compatible_crate
        .as_deref()
        .or(package_compatible)
    else {
        return Err(invalid_types_package(
            metadata_path,
            "compatible_crate",
            "configured metadata package must declare compatible_crate for consuming package version checks",
        ));
    };
    let Some(expected_version) = expected_version else {
        return Ok(());
    };
    if version_requirement_allows(requirement, expected_version) {
        return Ok(());
    }
    Err(DeclarationLookup::Invalid(DeclarationError {
        path: metadata_path.to_path_buf(),
        key: "compatible_crate",
        message: format!(
            "metadata package requirement `{requirement}` does not allow crate version `{expected_version}`"
        ),
    }))
}

fn invalid_types_package(
    metadata_path: &Path,
    key: &'static str,
    message: &str,
) -> DeclarationLookup {
    DeclarationLookup::Invalid(DeclarationError {
        path: metadata_path.to_path_buf(),
        key,
        message: message.to_owned(),
    })
}

pub(super) fn declaration_covers_call(facts: &DeclarationFacts, call_path: Option<&str>) -> bool {
    let Some(call_path) = call_path else {
        return false;
    };
    facts.functions.iter().any(|path| path == call_path)
}

pub(super) fn typed_call_has_replay_critical_effects(
    facts: &DeclarationFacts,
    call_path: Option<&str>,
) -> bool {
    let Some(call_path) = call_path else {
        return true;
    };
    let Some(function) = facts
        .function_facts
        .iter()
        .find(|fact| fact.path == call_path)
    else {
        return true;
    };
    !function.effects.is_empty()
        || function.simulation.as_deref().is_some_and(|simulation| {
            !matches!(simulation, "pure" | "deterministic" | "none" | "typed")
        })
        || function
            .determinism
            .as_deref()
            .is_some_and(|determinism| !matches!(determinism, "deterministic" | "pure"))
}

fn source_without_declared_hash(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("declaration_hash =")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn declaration_hash(source: &str) -> String {
    stable_hash(&source_without_declared_hash(source))
}

pub(super) fn activity_covers_call(
    facts: Option<&DeclarationFacts>,
    call_path: Option<&str>,
) -> bool {
    let Some(facts) = facts else {
        return false;
    };
    let Some(call_path) = call_path else {
        return false;
    };
    facts
        .activity_facts
        .iter()
        .any(|activity| activity.path == call_path && activity.is_complete())
}

pub(super) fn activity_fact_for_call<'a>(
    facts: &'a DeclarationFacts,
    call_path: Option<&str>,
) -> Option<&'a DeclarationActivityFact> {
    let call_path = call_path?;
    facts
        .activity_facts
        .iter()
        .find(|activity| activity.path == call_path)
}

impl DeclarationActivityFact {
    fn is_complete(&self) -> bool {
        self.retry
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            && self
                .idempotency
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            && self.result.as_deref() == Some("record")
            && self
                .compensation
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

fn declaration_type_paths(parsed: &toml::Value) -> Vec<String> {
    table_array_paths(parsed, "type")
}

fn declaration_type_facts(parsed: &toml::Value) -> Vec<DeclarationTypeFact> {
    parsed
        .get("type")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(DeclarationTypeFact {
                path: entry.get("path").and_then(toml::Value::as_str)?.to_owned(),
                kind: entry
                    .get("kind")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                resource: entry
                    .get("resource")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
                must_call: entry
                    .get("must_call")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                ownership: entry
                    .get("ownership")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                strict_ward: entry
                    .get("strict_ward")
                    .or_else(|| entry.get("strict"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn declaration_function_paths(parsed: &toml::Value) -> Vec<String> {
    table_array_paths(parsed, "function")
}

fn declaration_function_facts(parsed: &toml::Value) -> Vec<DeclarationFunctionFact> {
    parsed
        .get("function")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(DeclarationFunctionFact {
                path: entry.get("path").and_then(toml::Value::as_str)?.to_owned(),
                effects: entry
                    .get("effects")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                simulation: entry
                    .get("simulation")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                determinism: entry
                    .get("determinism")
                    .or_else(|| entry.get("deterministic"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                returns: entry
                    .get("returns")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                obligation: entry
                    .get("obligation")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                strict_ward: entry
                    .get("strict_ward")
                    .or_else(|| entry.get("strict"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn declaration_adapter_paths(parsed: &toml::Value) -> Vec<String> {
    table_array_paths(parsed, "adapter")
}

fn declaration_activity_facts(parsed: &toml::Value) -> Vec<DeclarationActivityFact> {
    parsed
        .get("activity")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(DeclarationActivityFact {
                path: entry.get("path").and_then(toml::Value::as_str)?.to_owned(),
                retry: entry
                    .get("retry")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                idempotency: entry
                    .get("idempotency")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                result: entry
                    .get("result")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                compensation: entry
                    .get("compensation")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn table_array_paths(parsed: &toml::Value, key: &str) -> Vec<String> {
    parsed
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            entry
                .get("path")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn declaration_effects(parsed: &toml::Value) -> Vec<String> {
    let mut effects = Vec::new();
    if let Some(functions) = parsed.get("function").and_then(toml::Value::as_array) {
        for function in functions {
            if let Some(path) = function.get("path").and_then(toml::Value::as_str) {
                effects.push(path.to_owned());
            }
            if let Some(items) = function.get("effects").and_then(toml::Value::as_array) {
                effects.extend(
                    items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
    }
    effects.sort();
    effects.dedup();
    effects
}

pub(super) fn stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
