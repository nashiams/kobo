use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

pub(super) const BUILTIN_REGISTRY_NAME: &str = "builtin-v0.11";
const LOCAL_REGISTRY_INDEX: &str = ".kobo/registry/index.toml";
const KOBO_REGISTRY_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Copy)]
struct BuiltinRegistryEntry {
    crate_name: &'static str,
    cargo_version: &'static str,
    types_package: Option<BuiltinRegistryMetadataPackage>,
    adapter_package: Option<BuiltinRegistryMetadataPackage>,
    public_types: &'static [&'static str],
    public_functions: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct BuiltinRegistryMetadataPackage {
    package: &'static str,
    version: &'static str,
}

pub(super) struct CommunityRegistryEntry {
    pub(super) crate_name: String,
    pub(super) cargo_version: String,
    pub(super) types_package: Option<RegistryMetadataPackage>,
    pub(super) adapter_package: Option<RegistryMetadataPackage>,
    pub(super) public_types: Vec<String>,
    pub(super) public_functions: Vec<String>,
    pub(super) registry: String,
    pub(super) source_path: Option<PathBuf>,
    pub(super) expected_source_name: String,
    pub(super) expected_source_version: String,
    source: RegistrySource,
    trust_policy: Option<String>,
    signed_by: Option<String>,
    summary: Option<RegistrySummary>,
}

#[derive(Clone)]
pub(super) struct RegistryMetadataPackage {
    package: String,
    version: String,
    checksum: Option<String>,
    compatible_crate: Option<String>,
    metadata_path: Option<PathBuf>,
    declaration_path: Option<String>,
    declaration_hash: Option<String>,
    adapter_runtime: Option<String>,
    capture: Option<String>,
}

#[derive(Clone)]
struct RegistrySummary {
    path: String,
    hash: String,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RegistrySource {
    Builtin,
    LocalIndex,
}

#[derive(Clone, Copy)]
enum MetadataKind {
    Types,
    Adapter,
}

struct MetadataPackageSelection {
    package: String,
    version: Option<String>,
    source: &'static str,
    registry: Option<String>,
    checksum: Option<String>,
    compatible_crate: Option<String>,
    metadata_path: Option<PathBuf>,
    declaration_path: Option<String>,
    declaration_hash: Option<String>,
    adapter_runtime: Option<String>,
    capture: Option<String>,
    trust_policy: Option<String>,
    signed_by: Option<String>,
    summary: Option<RegistrySummary>,
    validated: bool,
    description: &'static str,
}

struct SourceIdentity {
    name: String,
    version: String,
}

struct CargoDependencyImport {
    member: Option<String>,
    manifest_path: String,
    alias: String,
    package: String,
    section: &'static str,
    target: Option<String>,
    version: Option<String>,
    source: Option<String>,
    default_features: Option<bool>,
    features: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Semver {
    major: u64,
    minor: u64,
    patch: u64,
}

pub(super) fn cmd_add(
    crate_name: &str,
    features: Option<&str>,
    version: Option<&str>,
    path: Option<&Path>,
    git: Option<&str>,
    no_default_features: bool,
    dev: bool,
    build: bool,
    target: Option<&str>,
    manifest_path: Option<&Path>,
    member: Option<&str>,
) -> anyhow::Result<()> {
    let cargo_path = target_cargo_manifest(manifest_path, member)?;
    let source = fs::read_to_string(&cargo_path)
        .with_context(|| format!("failed to read {}", cargo_path.display()))?;
    let mut document = source
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", cargo_path.display()))?;
    let (crate_name, spec_version) = split_crate_spec(crate_name);
    let registry_root = cargo_path.parent().unwrap_or_else(|| Path::new("."));
    let registry_version =
        community_registry_entry_in(registry_root, &crate_name)?.map(|entry| entry.cargo_version);
    if version
        .or(spec_version.as_deref())
        .or(registry_version.as_deref())
        .is_none()
        && path.is_none()
        && git.is_none()
    {
        delegate_cargo_add(
            &cargo_path,
            &crate_name,
            features,
            no_default_features,
            dev,
            build,
            target,
        )?;
        eprintln!(
            "Delegated `{crate_name}` dependency resolution to Cargo for {}",
            cargo_path.display()
        );
        return Ok(());
    }
    let dependency = cargo_dependency_value(
        &crate_name,
        features,
        version
            .or(spec_version.as_deref())
            .or(registry_version.as_deref()),
        path,
        git,
        no_default_features,
    )?;
    let section = dependency_section_for_add(&document, dev, build, target)?;
    upsert_dependency(&mut document, section, &crate_name, dependency)?;
    fs::write(&cargo_path, document.to_string())
        .with_context(|| format!("failed to write {}", cargo_path.display()))?;
    eprintln!(
        "Added Cargo dependency `{crate_name}` to {}",
        cargo_path.display()
    );
    if let Some(entry) = community_registry_entry_in(registry_root, &crate_name)? {
        emit_optional_metadata_suggestion(&entry);
    }
    Ok(())
}

fn delegate_cargo_add(
    cargo_path: &Path,
    crate_name: &str,
    features: Option<&str>,
    no_default_features: bool,
    dev: bool,
    build: bool,
    target: Option<&str>,
) -> anyhow::Result<()> {
    let mut command = Command::new("cargo");
    command
        .arg("add")
        .arg(crate_name)
        .arg("--manifest-path")
        .arg(cargo_path);
    if let Some(features) = features {
        command.arg("--features").arg(features);
    }
    if no_default_features {
        command.arg("--no-default-features");
    }
    if dev {
        command.arg("--dev");
    }
    if build {
        command.arg("--build");
    }
    if let Some(target) = target {
        command.arg("--target").arg(target);
    }
    let output = command.output().with_context(|| {
        format!("K0128: failed to run Cargo dependency resolver for `{crate_name}`")
    })?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "K0128: Cargo could not add `{crate_name}` to {}: {}",
        cargo_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn cmd_add_types(crate_name: &str) -> anyhow::Result<()> {
    let package = metadata_package_for(MetadataKind::Types, crate_name, Path::new("."))?;
    append_kobo_metadata(MetadataKind::Types, crate_name, package)
}

pub(super) fn cmd_add_adapter(crate_name: &str) -> anyhow::Result<()> {
    let package = metadata_package_for(MetadataKind::Adapter, crate_name, Path::new("."))?;
    append_kobo_metadata(MetadataKind::Adapter, crate_name, package)
}

pub(super) fn community_registry_entry(
    crate_name: &str,
) -> anyhow::Result<Option<CommunityRegistryEntry>> {
    community_registry_entry_in(Path::new("."), crate_name)
}

pub(super) fn validate_registry_source_identity(
    entry: &CommunityRegistryEntry,
) -> anyhow::Result<()> {
    let Some(source_path) = entry.source_path.as_deref() else {
        return Ok(());
    };
    let identity = source_identity(source_path)?;
    if identity.name == entry.expected_source_name
        && identity.version == entry.expected_source_version
    {
        return Ok(());
    }
    anyhow::bail!(
        "K0128: registry source_path crate identity mismatch for `{}`: expected {} {}, found {} {}",
        entry.crate_name,
        entry.expected_source_name,
        entry.expected_source_version,
        identity.name,
        identity.version
    );
}

fn community_registry_entry_in(
    registry_root: &Path,
    crate_name: &str,
) -> anyhow::Result<Option<CommunityRegistryEntry>> {
    if let Some(entry) = local_registry_entry(registry_root, crate_name)? {
        return Ok(Some(entry));
    }
    Ok(builtin_registry_entries()
        .iter()
        .copied()
        .find(|entry| entry.crate_name == crate_name)
        .map(CommunityRegistryEntry::from_builtin))
}

pub(super) fn cmd_migrate_cargo_deps() -> anyhow::Result<()> {
    migrate_cargo_deps_into_kobo_toml()?;
    eprintln!("Migrated Cargo dependencies into optional Kobo ecosystem metadata");
    Ok(())
}

pub(super) fn migrate_cargo_deps_into_kobo_toml() -> anyhow::Result<usize> {
    let mut output =
        remove_managed_dependency_sections(&ensure_kobo_header(read_optional("Kobo.toml")?));
    let (dependencies, migration_source) =
        collect_manifest_dependency_imports_with_source(Path::new("Cargo.toml"), None)?;
    let dependency_count = dependencies.len();
    if dependencies.is_empty() {
        fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
        return Ok(0);
    }

    for dependency in dependencies {
        output.push_str("\n[[ecosystem.crate]]\n");
        output.push_str(&format!("name = \"{}\"\n", dependency.package));
        if let Some(member) = dependency.member.as_deref() {
            output.push_str(&format!("member = \"{member}\"\n"));
        }
        output.push_str(&format!("manifest = \"{}\"\n", dependency.manifest_path));
        if dependency.package != dependency.alias {
            output.push_str(&format!("alias = \"{}\"\n", dependency.alias));
        }
        output.push_str(&format!("section = \"{}\"\n", dependency.section));
        if let Some(target) = dependency.target.as_deref() {
            output.push_str(&format!("target = \"{target}\"\n"));
        }
        if let Some(version) = dependency.version.as_deref() {
            output.push_str(&format!("version = \"{version}\"\n"));
        }
        if let Some(source) = dependency.source.as_deref() {
            output.push_str(&format!("source = \"{source}\"\n"));
        }
        if let Some(default_features) = dependency.default_features {
            output.push_str(&format!("default_features = {default_features}\n"));
        }
        if !dependency.features.is_empty() {
            output.push_str(&format!(
                "features = [{}]\n",
                quoted_string_list(&dependency.features)
            ));
        }
        output.push_str(&format!("migration_source = \"{migration_source}\"\n"));
        output.push_str("managed_by = \"kobo migrate-cargo-deps\"\n");
        output.push_str("policy = \"opaque\"\n");
        output
            .push_str("reason = \"imported from Cargo dependency; review before exact replay\"\n");
    }

    fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
    Ok(dependency_count)
}

fn remove_managed_dependency_sections(source: &str) -> String {
    let mut kept_blocks = Vec::new();
    let mut current = Vec::new();
    for line in source.lines() {
        if line.trim() == "[[ecosystem.crate]]" && !current.is_empty() {
            push_if_unmanaged(&mut kept_blocks, &current);
            current.clear();
        }
        current.push(line.to_owned());
    }
    if !current.is_empty() {
        push_if_unmanaged(&mut kept_blocks, &current);
    }
    let mut output = kept_blocks.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn push_if_unmanaged(output: &mut Vec<String>, block: &[String]) {
    let block_text = block.join("\n");
    let is_managed_ecosystem_crate = block
        .first()
        .is_some_and(|line| line.trim() == "[[ecosystem.crate]]")
        && (block_text.contains("managed_by = \"kobo migrate-cargo-deps\"")
            || block_text.contains(
                "reason = \"imported from Cargo dependency; review before exact replay\"",
            ));
    if !is_managed_ecosystem_crate {
        output.push(block_text);
    }
}

fn cargo_dependency_value(
    crate_name: &str,
    features: Option<&str>,
    version: Option<&str>,
    path: Option<&Path>,
    git: Option<&str>,
    no_default_features: bool,
) -> anyhow::Result<Item> {
    let mut fields = InlineTable::new();
    if let Some(path) = path {
        fields.insert(
            "path",
            Value::from(path.display().to_string().replace('\\', "/")),
        );
    } else if let Some(git) = git {
        fields.insert("git", Value::from(git));
        if let Some(version) = version {
            fields.insert("version", Value::from(version));
        }
    } else if let Some(version) = version {
        fields.insert("version", Value::from(version));
    } else {
        anyhow::bail!(
            "K0128: kobo add needs an explicit version, path, git, or crate@version when Cargo metadata is unavailable"
        );
    }
    if no_default_features {
        fields.insert("default-features", Value::from(false));
    }
    if let Some(features) = features {
        let mut feature_array = Array::default();
        for feature in features
            .split(',')
            .map(str::trim)
            .filter(|feature| !feature.is_empty())
        {
            feature_array.push(feature);
        }
        if !feature_array.is_empty() {
            fields.insert("features", Value::Array(feature_array));
        }
    }
    let _ = crate_name;
    Ok(Item::Value(Value::InlineTable(fields)))
}

fn builtin_registry_entries() -> &'static [BuiltinRegistryEntry] {
    &[
        BuiltinRegistryEntry {
            crate_name: "serde_json",
            cargo_version: "1",
            types_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-types-serde-json",
                version: "1",
            }),
            adapter_package: None,
            public_types: &["Value", "Map", "Number"],
            public_functions: &["from_str", "to_string", "to_vec"],
        },
        BuiltinRegistryEntry {
            crate_name: "reqwest",
            cargo_version: "0.12",
            types_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-types-reqwest",
                version: "0.12",
            }),
            adapter_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-adapter-reqwest",
                version: "0.12",
            }),
            public_types: &["Client", "RequestBuilder", "Response", "Error"],
            public_functions: &["get"],
        },
        BuiltinRegistryEntry {
            crate_name: "sqlx",
            cargo_version: "0.8",
            types_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-types-sqlx",
                version: "0.8",
            }),
            adapter_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-adapter-sqlx",
                version: "0.8",
            }),
            public_types: &["Pool", "Transaction", "Executor", "Row"],
            public_functions: &["query", "query_as"],
        },
        BuiltinRegistryEntry {
            crate_name: "tokio",
            cargo_version: "1",
            types_package: None,
            adapter_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-adapter-tokio",
                version: "1",
            }),
            public_types: &["task::JoinHandle", "runtime::Runtime"],
            public_functions: &["spawn", "select"],
        },
        BuiltinRegistryEntry {
            crate_name: "anyhow",
            cargo_version: "1",
            types_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-types-anyhow",
                version: "1",
            }),
            adapter_package: None,
            public_types: &["Error", "Result"],
            public_functions: &["anyhow"],
        },
        BuiltinRegistryEntry {
            crate_name: "clap",
            cargo_version: "4",
            types_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-types-clap",
                version: "4",
            }),
            adapter_package: None,
            public_types: &["Command", "Arg"],
            public_functions: &[],
        },
        BuiltinRegistryEntry {
            crate_name: "thiserror",
            cargo_version: "2",
            types_package: Some(BuiltinRegistryMetadataPackage {
                package: "kobo-types-thiserror",
                version: "2",
            }),
            adapter_package: None,
            public_types: &["Error"],
            public_functions: &[],
        },
    ]
}

impl CommunityRegistryEntry {
    fn from_builtin(entry: BuiltinRegistryEntry) -> Self {
        Self {
            crate_name: entry.crate_name.to_owned(),
            cargo_version: entry.cargo_version.to_owned(),
            types_package: entry
                .types_package
                .map(RegistryMetadataPackage::from_builtin),
            adapter_package: entry
                .adapter_package
                .map(RegistryMetadataPackage::from_builtin),
            public_types: entry
                .public_types
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            public_functions: entry
                .public_functions
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            registry: BUILTIN_REGISTRY_NAME.to_owned(),
            source_path: None,
            expected_source_name: entry.crate_name.to_owned(),
            expected_source_version: entry.cargo_version.to_owned(),
            source: RegistrySource::Builtin,
            trust_policy: Some("builtin-reviewed".to_owned()),
            signed_by: Some("kobo-core".to_owned()),
            summary: None,
        }
    }
}

impl RegistryMetadataPackage {
    fn from_builtin(package: BuiltinRegistryMetadataPackage) -> Self {
        Self {
            package: package.package.to_owned(),
            version: package.version.to_owned(),
            checksum: None,
            compatible_crate: None,
            metadata_path: None,
            declaration_path: None,
            declaration_hash: None,
            adapter_runtime: Some(format!("kobo-builtin-{}", package.package)),
            capture: Some("facade-call-capture".to_owned()),
        }
    }
}

fn local_registry_entry(
    registry_root: &Path,
    crate_name: &str,
) -> anyhow::Result<Option<CommunityRegistryEntry>> {
    let registry_path = registry_root.join(LOCAL_REGISTRY_INDEX);
    if !registry_path.is_file() {
        return Ok(None);
    }
    let source = fs::read_to_string(&registry_path)
        .with_context(|| format!("failed to read {}", registry_path.display()))?;
    let document: toml::Value = toml::from_str(&source)
        .with_context(|| format!("failed to parse {}", registry_path.display()))?;
    validate_registry_header(&document, &registry_path)?;
    let registry_name = document
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or("local-v0.11")
        .to_owned();
    let Some(entries) = document.get("crate").and_then(toml::Value::as_array) else {
        return Ok(None);
    };
    for entry in entries {
        if entry.get("name").and_then(toml::Value::as_str) != Some(crate_name) {
            continue;
        }
        return Ok(Some(parse_local_registry_entry(
            entry,
            &registry_name,
            &registry_path,
        )?));
    }
    Ok(None)
}

fn validate_registry_header(document: &toml::Value, registry_path: &Path) -> anyhow::Result<()> {
    let schema_version = document
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if schema_version != KOBO_REGISTRY_SCHEMA_VERSION {
        anyhow::bail!(
            "K0128: registry index {} uses unsupported schema_version {schema_version}",
            registry_path.display()
        );
    }
    if document
        .get("trust_policy")
        .and_then(toml::Value::as_str)
        .is_none_or(|policy| policy != "workspace-pinned")
    {
        anyhow::bail!(
            "K0128: registry index {} is not trusted by this project; expected trust_policy = \"workspace-pinned\"",
            registry_path.display()
        );
    }
    Ok(())
}

fn parse_local_registry_entry(
    entry: &toml::Value,
    registry_name: &str,
    registry_path: &Path,
) -> anyhow::Result<CommunityRegistryEntry> {
    let crate_name = required_registry_str(entry, "name", registry_path)?;
    let cargo_version = required_registry_str(entry, "cargo_version", registry_path)?;
    let signed_by = required_registry_str(entry, "signed_by", registry_path)?;
    validate_registry_signer(entry, signed_by, registry_path)?;
    let registry_root = registry_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    let source_path = entry
        .get("source_path")
        .and_then(toml::Value::as_str)
        .map(|path| confined_registry_source_path(registry_root, path))
        .transpose()?;
    let summary = registry_summary(entry, registry_path)?;
    Ok(CommunityRegistryEntry {
        crate_name: crate_name.to_owned(),
        cargo_version: cargo_version.to_owned(),
        types_package: registry_metadata_package(
            entry,
            "types",
            cargo_version,
            signed_by,
            registry_path,
        )?,
        adapter_package: registry_metadata_package(
            entry,
            "adapter",
            cargo_version,
            signed_by,
            registry_path,
        )?,
        public_types: registry_string_array(entry, "public_types"),
        public_functions: registry_string_array(entry, "public_functions"),
        registry: registry_name.to_owned(),
        source_path,
        expected_source_name: crate_name.to_owned(),
        expected_source_version: cargo_version.to_owned(),
        source: RegistrySource::LocalIndex,
        trust_policy: Some("workspace-pinned".to_owned()),
        signed_by: Some(signed_by.to_owned()),
        summary,
    })
}

fn validate_registry_signer(
    entry: &toml::Value,
    signed_by: &str,
    registry_path: &Path,
) -> anyhow::Result<()> {
    let Some(document) = entry.get("__document") else {
        return validate_registry_signer_from_file(signed_by, registry_path);
    };
    let _ = document;
    validate_registry_signer_from_file(signed_by, registry_path)
}

fn validate_registry_signer_from_file(signed_by: &str, registry_path: &Path) -> anyhow::Result<()> {
    let source = fs::read_to_string(registry_path)
        .with_context(|| format!("failed to read {}", registry_path.display()))?;
    let document: toml::Value = toml::from_str(&source)
        .with_context(|| format!("failed to parse {}", registry_path.display()))?;
    let trusted = document
        .get("trusted_signers")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .any(|signer| signer == signed_by);
    if trusted {
        return Ok(());
    }
    anyhow::bail!(
        "K0128: registry entry signer `{signed_by}` is not trusted by {}",
        registry_path.display()
    );
}

fn confined_registry_source_path(registry_root: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let configured = Path::new(path);
    if configured.is_absolute()
        || configured
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        anyhow::bail!("K0128: registry source_path escapes workspace: {path}");
    }
    let registry_root = if registry_root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        registry_root
    };
    let root = registry_root.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize registry root {}",
            registry_root.display()
        )
    })?;
    let source_path = registry_root.join(path);
    if !source_path.is_dir() {
        anyhow::bail!(
            "K0128: registry source_path must be an existing crate directory: {}",
            source_path.display()
        );
    }
    let canonical = source_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize registry source_path {}",
            source_path.display()
        )
    })?;
    if canonical.starts_with(&root) {
        return Ok(source_path);
    }
    anyhow::bail!(
        "K0128: registry source_path escapes workspace: {}",
        source_path.display()
    );
}

fn source_identity(source_path: &Path) -> anyhow::Result<SourceIdentity> {
    let manifest_path = source_path.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: toml::Value = toml::from_str(&source)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .with_context(|| format!("{} is missing [package]", manifest_path.display()))?;
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} is missing package.name", manifest_path.display()))?;
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} is missing package.version", manifest_path.display()))?;
    Ok(SourceIdentity {
        name: name.to_owned(),
        version: version.to_owned(),
    })
}

fn required_registry_str<'a>(
    value: &'a toml::Value,
    key: &str,
    registry_path: &Path,
) -> anyhow::Result<&'a str> {
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .with_context(|| {
            format!(
                "K0128: registry index {} is missing `{key}`",
                registry_path.display()
            )
        })
}

fn registry_string_array(value: &toml::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn registry_summary(
    entry: &toml::Value,
    registry_path: &Path,
) -> anyhow::Result<Option<RegistrySummary>> {
    let Some(path) = entry.get("summary_path").and_then(toml::Value::as_str) else {
        return Ok(None);
    };
    let hash = required_registry_str(entry, "summary_hash", registry_path)?;
    validate_sha256_literal(hash, "summary_hash", registry_path)?;
    Ok(Some(RegistrySummary {
        path: path.to_owned(),
        hash: hash.to_owned(),
    }))
}

fn registry_metadata_package(
    entry: &toml::Value,
    key: &str,
    cargo_version: &str,
    signed_by: &str,
    registry_path: &Path,
) -> anyhow::Result<Option<RegistryMetadataPackage>> {
    let Some(package) = entry.get(key).and_then(toml::Value::as_table) else {
        return Ok(None);
    };
    let package_name = required_table_str(package, "package", key, registry_path)?;
    let version = required_table_str(package, "version", key, registry_path)?;
    let checksum = required_table_str(package, "checksum", key, registry_path)?;
    validate_sha256_literal(checksum, "checksum", registry_path)?;
    let metadata_path = package
        .get("metadata_path")
        .and_then(toml::Value::as_str)
        .map(|path| {
            registry_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(path)
        });
    if let Some(metadata_path) = metadata_path.as_deref() {
        validate_file_sha256(metadata_path, checksum, registry_path)?;
    }
    let compatible_crate = package
        .get("compatible_crate")
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    if let Some(requirement) = compatible_crate.as_deref() {
        validate_version_requirement(requirement, cargo_version, registry_path)?;
    }
    let details = if let Some(metadata_path) = metadata_path.as_deref() {
        Some(validate_metadata_package_file(
            metadata_path,
            key,
            package_name,
            version,
            compatible_crate.as_deref(),
            signed_by,
            registry_path,
        )?)
    } else {
        None
    };
    Ok(Some(RegistryMetadataPackage {
        package: package_name.to_owned(),
        version: version.to_owned(),
        checksum: Some(checksum.to_owned()),
        compatible_crate,
        metadata_path,
        declaration_path: details
            .as_ref()
            .and_then(|details| details.declaration_path.clone()),
        declaration_hash: details
            .as_ref()
            .and_then(|details| details.declaration_hash.clone()),
        adapter_runtime: details
            .as_ref()
            .and_then(|details| details.adapter_runtime.clone()),
        capture: details.and_then(|details| details.capture),
    }))
}

struct MetadataPackageDetails {
    declaration_path: Option<String>,
    declaration_hash: Option<String>,
    adapter_runtime: Option<String>,
    capture: Option<String>,
}

fn validate_metadata_package_file(
    metadata_path: &Path,
    key: &str,
    package_name: &str,
    version: &str,
    compatible_crate: Option<&str>,
    signed_by: &str,
    registry_path: &Path,
) -> anyhow::Result<MetadataPackageDetails> {
    let source = fs::read_to_string(metadata_path).with_context(|| {
        format!(
            "K0128: registry index {} points to unreadable metadata package {}",
            registry_path.display(),
            metadata_path.display()
        )
    })?;
    let parsed: toml::Value = toml::from_str(&source).with_context(|| {
        format!(
            "K0128: registry metadata package {} is not valid TOML",
            metadata_path.display()
        )
    })?;
    let schema_version = parsed
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if schema_version != KOBO_REGISTRY_SCHEMA_VERSION {
        anyhow::bail!(
            "K0128: registry metadata package {} uses unsupported schema_version {schema_version}",
            metadata_path.display()
        );
    }
    if parsed.get("kind").and_then(toml::Value::as_str) != Some(key) {
        anyhow::bail!(
            "K0128: registry metadata package {} must declare kind = \"{}\"",
            metadata_path.display(),
            key
        );
    }
    if parsed.get("package").and_then(toml::Value::as_str) != Some(package_name) {
        anyhow::bail!(
            "K0128: registry metadata package {} does not describe package `{package_name}`",
            metadata_path.display()
        );
    }
    if parsed.get("version").and_then(toml::Value::as_str) != Some(version) {
        anyhow::bail!(
            "K0128: registry metadata package {} has version mismatch for `{package_name}`",
            metadata_path.display()
        );
    }
    if parsed.get("signed_by").and_then(toml::Value::as_str) != Some(signed_by) {
        anyhow::bail!(
            "K0128: registry metadata package {} must be signed by trusted signer `{signed_by}`",
            metadata_path.display()
        );
    }
    if let Some(requirement) = compatible_crate {
        if parsed.get("compatible_crate").and_then(toml::Value::as_str) != Some(requirement) {
            anyhow::bail!(
                "K0128: registry metadata package {} has incompatible compatible_crate declaration",
                metadata_path.display()
            );
        }
    }
    match key {
        "types" => validate_types_metadata_package(&parsed, metadata_path, registry_path),
        "adapter" => validate_adapter_metadata_package(&parsed, metadata_path),
        _ => anyhow::bail!("K0128: unsupported registry metadata package kind `{key}`"),
    }
}

fn validate_types_metadata_package(
    parsed: &toml::Value,
    metadata_path: &Path,
    _registry_path: &Path,
) -> anyhow::Result<MetadataPackageDetails> {
    let declaration_path =
        required_metadata_package_str(parsed, "declaration_path", metadata_path)?.to_owned();
    let declaration_hash =
        required_metadata_package_str(parsed, "declaration_hash", metadata_path)?.to_owned();
    let resolved_path = if Path::new(&declaration_path).is_absolute() {
        PathBuf::from(&declaration_path)
    } else {
        metadata_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&declaration_path)
    };
    let declaration_source = fs::read_to_string(&resolved_path).with_context(|| {
        format!(
            "K0128: registry metadata package {} points to missing declaration {}",
            metadata_path.display(),
            resolved_path.display()
        )
    })?;
    let actual_hash = super::declarations::declaration_hash(&declaration_source);
    if actual_hash != declaration_hash {
        anyhow::bail!(
            "K0128: registry metadata package {} declaration_hash mismatch for {}: expected {}, found {}",
            metadata_path.display(),
            resolved_path.display(),
            declaration_hash,
            actual_hash
        );
    }
    Ok(MetadataPackageDetails {
        declaration_path: Some(declaration_path),
        declaration_hash: Some(declaration_hash),
        adapter_runtime: None,
        capture: None,
    })
}

fn validate_adapter_metadata_package(
    parsed: &toml::Value,
    metadata_path: &Path,
) -> anyhow::Result<MetadataPackageDetails> {
    let adapter_runtime =
        required_metadata_package_str(parsed, "adapter_runtime", metadata_path)?.to_owned();
    let capture = required_metadata_package_str(parsed, "capture", metadata_path)?.to_owned();
    if capture != "boundary-io" && capture != "facade-call-capture" {
        anyhow::bail!(
            "K0128: registry adapter package {} has unsupported capture mode `{capture}`",
            metadata_path.display()
        );
    }
    Ok(MetadataPackageDetails {
        declaration_path: None,
        declaration_hash: None,
        adapter_runtime: Some(adapter_runtime),
        capture: Some(capture),
    })
}

fn required_metadata_package_str<'a>(
    value: &'a toml::Value,
    key: &str,
    metadata_path: &Path,
) -> anyhow::Result<&'a str> {
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .with_context(|| {
            format!(
                "K0128: registry metadata package {} is missing `{key}`",
                metadata_path.display()
            )
        })
}

fn required_table_str<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    field: &str,
    section: &str,
    registry_path: &Path,
) -> anyhow::Result<&'a str> {
    table
        .get(field)
        .and_then(toml::Value::as_str)
        .with_context(|| {
            format!(
                "K0128: registry index {} is missing `{section}.{field}`",
                registry_path.display()
            )
        })
}

fn validate_sha256_literal(value: &str, field: &str, registry_path: &Path) -> anyhow::Result<()> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        anyhow::bail!(
            "K0128: registry index {} has invalid {field}; expected sha256:<digest>",
            registry_path.display()
        );
    };
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    anyhow::bail!(
        "K0128: registry index {} has invalid {field}; expected sha256:<64 hex digits>",
        registry_path.display()
    );
}

fn validate_file_sha256(
    metadata_path: &Path,
    expected: &str,
    registry_path: &Path,
) -> anyhow::Result<()> {
    let source = fs::read(metadata_path).with_context(|| {
        format!(
            "K0128: registry index {} points to missing metadata package {}",
            registry_path.display(),
            metadata_path.display()
        )
    })?;
    let actual = format!("sha256:{}", sha256_hex(&source));
    if actual == expected {
        return Ok(());
    }
    anyhow::bail!(
        "K0128: registry metadata package checksum mismatch for {}: expected {}, found {}",
        metadata_path.display(),
        expected,
        actual
    );
}

fn validate_version_requirement(
    requirement: &str,
    version: &str,
    registry_path: &Path,
) -> anyhow::Result<()> {
    if version_requirement_allows(requirement, version) {
        return Ok(());
    }
    anyhow::bail!(
        "K0128: registry index {} has incompatible crate version {version} for {requirement}",
        registry_path.display()
    );
}

pub(super) fn version_requirement_allows(requirement: &str, version: &str) -> bool {
    requirement
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .all(|part| version_matches_requirement_part(part, version))
}

fn version_matches_requirement_part(requirement: &str, version: &str) -> bool {
    if let Some(minimum) = requirement.strip_prefix(">=") {
        return semver_allows(version, minimum, |left, right| left >= right);
    }
    if let Some(minimum) = requirement.strip_prefix('>') {
        return semver_allows(version, minimum, |left, right| left > right);
    }
    if let Some(maximum) = requirement.strip_prefix("<=") {
        return semver_allows(version, maximum, |left, right| left <= right);
    }
    if let Some(maximum) = requirement.strip_prefix('<') {
        return semver_allows(version, maximum, |left, right| left < right);
    }
    if let Some(exact) = requirement.strip_prefix('=') {
        return semver_allows(version, exact, |left, right| left == right);
    }
    if let Some(base) = requirement.strip_prefix('^') {
        return caret_requirement_allows(version, base);
    }
    if let Some(base) = requirement.strip_prefix('~') {
        return tilde_requirement_allows(version, base);
    }
    semver_allows(version, requirement, |left, right| left == right)
}

fn semver_allows(
    version: &str,
    requirement: &str,
    comparator: impl FnOnce(Semver, Semver) -> bool,
) -> bool {
    let Some(version) = Semver::parse(version) else {
        return false;
    };
    let Some(requirement) = Semver::parse(requirement) else {
        return false;
    };
    comparator(version, requirement)
}

fn caret_requirement_allows(version: &str, base: &str) -> bool {
    let Some(version) = Semver::parse(version) else {
        return false;
    };
    let Some(base) = Semver::parse(base) else {
        return false;
    };
    let upper = if base.major > 0 {
        Semver {
            major: base.major + 1,
            minor: 0,
            patch: 0,
        }
    } else if base.minor > 0 {
        Semver {
            major: 0,
            minor: base.minor + 1,
            patch: 0,
        }
    } else {
        Semver {
            major: 0,
            minor: 0,
            patch: base.patch + 1,
        }
    };
    version >= base && version < upper
}

fn tilde_requirement_allows(version: &str, base: &str) -> bool {
    let Some(version) = Semver::parse(version) else {
        return false;
    };
    let Some(base) = Semver::parse(base) else {
        return false;
    };
    let upper = Semver {
        major: base.major,
        minor: base.minor + 1,
        patch: 0,
    };
    version >= base && version < upper
}

impl Semver {
    fn parse(value: &str) -> Option<Self> {
        let core = value
            .split_once('-')
            .map(|(core, _)| core)
            .unwrap_or(value)
            .split_once('+')
            .map(|(core, _)| core)
            .unwrap_or(value);
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bit_len = (message.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks(64) {
        let mut schedule = [0_u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }
        let mut work = hash;
        for index in 0..64 {
            let sum1 =
                work[4].rotate_right(6) ^ work[4].rotate_right(11) ^ work[4].rotate_right(25);
            let choose = (work[4] & work[5]) ^ (!work[4] & work[6]);
            let temp1 = work[7]
                .wrapping_add(sum1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let sum0 =
                work[0].rotate_right(2) ^ work[0].rotate_right(13) ^ work[0].rotate_right(22);
            let majority = (work[0] & work[1]) ^ (work[0] & work[2]) ^ (work[1] & work[2]);
            let temp2 = sum0.wrapping_add(majority);
            work = [
                temp1.wrapping_add(temp2),
                work[0],
                work[1],
                work[2],
                work[3].wrapping_add(temp1),
                work[4],
                work[5],
                work[6],
            ];
        }
        for (slot, value) in hash.iter_mut().zip(work) {
            *slot = slot.wrapping_add(value);
        }
    }
    hash.iter()
        .map(|word| format!("{word:08x}"))
        .collect::<String>()
}

fn metadata_package_for(
    kind: MetadataKind,
    crate_name: &str,
    registry_root: &Path,
) -> anyhow::Result<MetadataPackageSelection> {
    if let Some(entry) = community_registry_entry_in(registry_root, crate_name)? {
        let registry_package = match kind {
            MetadataKind::Types => entry.types_package.clone(),
            MetadataKind::Adapter => entry.adapter_package.clone(),
        };
        if let Some(package) = registry_package {
            if let Some(consuming_version) = consuming_dependency_version(crate_name)? {
                validate_version_requirement(
                    package
                        .compatible_crate
                        .as_deref()
                        .unwrap_or(&entry.cargo_version),
                    &consuming_version,
                    &registry_root.join(LOCAL_REGISTRY_INDEX),
                )
                .with_context(|| {
                    format!(
                        "K0128: incompatible consuming dependency version {consuming_version} for `{crate_name}`"
                    )
                })?;
            }
            let source = if entry.source == RegistrySource::LocalIndex {
                "registry-index"
            } else {
                "registry"
            };
            return Ok(MetadataPackageSelection {
                package: package.package,
                version: Some(package.version),
                source,
                registry: Some(entry.registry),
                checksum: package.checksum,
                compatible_crate: package.compatible_crate,
                metadata_path: package.metadata_path,
                declaration_path: package.declaration_path,
                declaration_hash: package.declaration_hash,
                adapter_runtime: package.adapter_runtime,
                capture: package.capture,
                trust_policy: entry.trust_policy,
                signed_by: entry.signed_by,
                summary: entry.summary,
                validated: true,
                description: kind.description(),
            });
        }
    }
    Ok(MetadataPackageSelection {
        package: format!("{}-{crate_name}", kind.default_package_prefix()),
        version: None,
        source: "convention",
        registry: None,
        checksum: None,
        compatible_crate: None,
        metadata_path: None,
        declaration_path: None,
        declaration_hash: None,
        adapter_runtime: None,
        capture: None,
        trust_policy: None,
        signed_by: None,
        summary: None,
        validated: false,
        description: kind.description(),
    })
}

impl MetadataKind {
    fn section_name(self) -> &'static str {
        match self {
            Self::Types => "types",
            Self::Adapter => "adapter",
        }
    }

    fn default_package_prefix(self) -> &'static str {
        match self {
            Self::Types => "kobo-types",
            Self::Adapter => "kobo-adapter",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Types => "declaration metadata package",
            Self::Adapter => "simulation adapter package",
        }
    }
}

fn emit_optional_metadata_suggestion(entry: &CommunityRegistryEntry) {
    let mut packages = Vec::new();
    if let Some(package) = entry.types_package.as_ref() {
        packages.push(format!(
            "declaration metadata package: {} {}",
            package.package, package.version
        ));
    }
    if let Some(package) = entry.adapter_package.as_ref() {
        packages.push(format!(
            "simulation adapter package: {} {}",
            package.package, package.version
        ));
    }
    if packages.is_empty() {
        return;
    }
    eprintln!(
        "optional Kobo metadata available for `{}`:\n  {}\nAdd now? [types] [types+adapter] [skip]",
        entry.crate_name,
        packages.join("\n  ")
    );
    let mut commands = Vec::new();
    if entry.types_package.is_some() {
        commands.push(format!("kobo add-types {}", entry.crate_name));
    }
    if entry.adapter_package.is_some() {
        commands.push(format!("kobo add-adapter {}", entry.crate_name));
    }
    if !commands.is_empty() {
        eprintln!("noninteractive: run `{}`", commands.join("` or `"));
    }
}

fn consuming_dependency_version(crate_name: &str) -> anyhow::Result<Option<String>> {
    let dependencies = collect_manifest_dependency_imports(Path::new("Cargo.toml"), None)?;
    Ok(dependencies
        .into_iter()
        .find(|dependency| dependency.package == crate_name || dependency.alias == crate_name)
        .and_then(|dependency| dependency.version))
}

fn collect_manifest_dependency_imports(
    manifest_path: &Path,
    member: Option<&str>,
) -> anyhow::Result<Vec<CargoDependencyImport>> {
    Ok(collect_manifest_dependency_imports_with_source(manifest_path, member)?.0)
}

fn collect_manifest_dependency_imports_with_source(
    manifest_path: &Path,
    member: Option<&str>,
) -> anyhow::Result<(Vec<CargoDependencyImport>, &'static str)> {
    if member.is_none() {
        if let Some(imports) =
            collect_manifest_dependency_imports_from_cargo_metadata(manifest_path)
        {
            return Ok((imports, "cargo-metadata"));
        }
    }
    Ok((
        collect_manifest_dependency_imports_by_manifest_scan(manifest_path, member)?,
        "manifest-scan",
    ))
}

fn collect_manifest_dependency_imports_by_manifest_scan(
    manifest_path: &Path,
    member: Option<&str>,
) -> anyhow::Result<Vec<CargoDependencyImport>> {
    let cargo_source = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let cargo: toml::Value = toml::from_str(&cargo_source)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let manifest_label = manifest_path.display().to_string().replace('\\', "/");
    let mut imports = collect_cargo_dependency_imports(&cargo, member, &manifest_label);
    if member.is_none() {
        for workspace_member in workspace_members(&cargo) {
            let member_manifest = Path::new(&workspace_member).join("Cargo.toml");
            if !member_manifest.is_file() {
                continue;
            }
            imports.extend(collect_manifest_dependency_imports(
                &member_manifest,
                Some(&workspace_member),
            )?);
        }
    }
    Ok(imports)
}

fn collect_manifest_dependency_imports_from_cargo_metadata(
    manifest_path: &Path,
) -> Option<Vec<CargoDependencyImport>> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .arg("--manifest-path")
        .arg(manifest_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let root_manifest = absolutize_path(manifest_path);
    let root_dir = root_manifest.parent().unwrap_or_else(|| Path::new("."));
    let packages = metadata.get("packages")?.as_array()?;
    let mut imports = Vec::new();
    for package in packages {
        let manifest = package
            .get("manifest_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)?;
        let manifest_abs = absolutize_path(&manifest);
        let member = if manifest_abs == root_manifest {
            None
        } else {
            manifest_abs
                .parent()
                .and_then(|dir| dir.strip_prefix(root_dir).ok())
                .map(|dir| dir.display().to_string().replace('\\', "/"))
        };
        let label = manifest_abs
            .strip_prefix(root_dir)
            .map(|path| path.display().to_string().replace('\\', "/"))
            .unwrap_or_else(|_| manifest.display().to_string().replace('\\', "/"));
        imports.extend(collect_cargo_metadata_dependency_imports(
            package,
            member.as_deref(),
            &label,
        )?);
    }
    imports.sort_by(|left, right| {
        (
            left.member.as_deref(),
            left.manifest_path.as_str(),
            left.section,
            left.target.as_deref(),
            left.alias.as_str(),
        )
            .cmp(&(
                right.member.as_deref(),
                right.manifest_path.as_str(),
                right.section,
                right.target.as_deref(),
                right.alias.as_str(),
            ))
    });
    imports.dedup_by(|left, right| {
        left.member == right.member
            && left.manifest_path == right.manifest_path
            && left.section == right.section
            && left.target == right.target
            && left.alias == right.alias
            && left.package == right.package
    });
    Some(imports)
}

fn collect_cargo_metadata_dependency_imports(
    package: &serde_json::Value,
    member: Option<&str>,
    manifest_path: &str,
) -> Option<Vec<CargoDependencyImport>> {
    let dependencies = package.get("dependencies")?.as_array()?;
    let mut imports = Vec::new();
    for dependency in dependencies {
        let package_name = dependency
            .get("name")
            .and_then(serde_json::Value::as_str)?
            .to_owned();
        let alias = dependency
            .get("rename")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(package_name.as_str())
            .to_owned();
        let kind = dependency.get("kind").and_then(serde_json::Value::as_str);
        let target = dependency
            .get("target")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let section = cargo_metadata_dependency_section(kind, target.as_deref());
        let version = dependency
            .get("req")
            .and_then(serde_json::Value::as_str)
            .filter(|req| !req.is_empty() && *req != "*")
            .map(trim_cargo_requirement);
        let source = cargo_metadata_dependency_source(dependency);
        let default_features = dependency
            .get("uses_default_features")
            .and_then(serde_json::Value::as_bool);
        let features = dependency
            .get("features")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect();
        imports.push(CargoDependencyImport {
            member: member.map(str::to_owned),
            manifest_path: manifest_path.to_owned(),
            alias,
            package: package_name,
            section,
            target,
            version,
            source,
            default_features,
            features,
        });
    }
    Some(imports)
}

fn cargo_metadata_dependency_section(kind: Option<&str>, target: Option<&str>) -> &'static str {
    match (target.is_some(), kind) {
        (true, Some("dev")) => "target.dev-dependencies",
        (true, Some("build")) => "target.build-dependencies",
        (true, _) => "target.dependencies",
        (false, Some("dev")) => "dev-dependencies",
        (false, Some("build")) => "build-dependencies",
        (false, _) => "dependencies",
    }
}

fn trim_cargo_requirement(requirement: &str) -> String {
    requirement
        .strip_prefix('^')
        .or_else(|| requirement.strip_prefix('='))
        .unwrap_or(requirement)
        .to_owned()
}

fn cargo_metadata_dependency_source(dependency: &serde_json::Value) -> Option<String> {
    let source = dependency.get("source").and_then(serde_json::Value::as_str);
    if source.is_some_and(|source| source.starts_with("registry+")) {
        return Some("registry".to_owned());
    }
    if source.is_some_and(|source| source.starts_with("git+")) {
        return source.map(str::to_owned);
    }
    dependency
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(|path| format!("path:{path}"))
        .or_else(|| source.map(str::to_owned))
}

fn absolutize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn collect_cargo_dependency_imports(
    cargo: &toml::Value,
    member: Option<&str>,
    manifest_path: &str,
) -> Vec<CargoDependencyImport> {
    let mut imports = Vec::new();
    imports.extend(dependencies_in_section(
        cargo,
        "dependencies",
        "dependencies",
        None,
        member,
        manifest_path,
    ));
    imports.extend(dependencies_in_section(
        cargo,
        "dev-dependencies",
        "dev-dependencies",
        None,
        member,
        manifest_path,
    ));
    imports.extend(dependencies_in_section(
        cargo,
        "build-dependencies",
        "build-dependencies",
        None,
        member,
        manifest_path,
    ));
    if let Some(targets) = cargo.get("target").and_then(toml::Value::as_table) {
        for (target, target_config) in targets {
            imports.extend(dependencies_in_section(
                target_config,
                "dependencies",
                "target.dependencies",
                Some(target.clone()),
                member,
                manifest_path,
            ));
            imports.extend(dependencies_in_section(
                target_config,
                "dev-dependencies",
                "target.dev-dependencies",
                Some(target.clone()),
                member,
                manifest_path,
            ));
            imports.extend(dependencies_in_section(
                target_config,
                "build-dependencies",
                "target.build-dependencies",
                Some(target.clone()),
                member,
                manifest_path,
            ));
        }
    }
    imports
}

fn workspace_members(cargo: &toml::Value) -> Vec<String> {
    let members = cargo
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    let excludes = cargo
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("exclude"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    let mut resolved = Vec::new();
    for member in members {
        resolved.extend(resolve_workspace_member_pattern(member));
    }
    resolved.sort();
    resolved.dedup();
    resolved
        .into_iter()
        .filter(|member| !excludes.iter().any(|exclude| member == exclude))
        .collect()
}

fn resolve_workspace_member_pattern(pattern: &str) -> Vec<String> {
    let Some(prefix) = pattern.strip_suffix("/*") else {
        return vec![pattern.to_owned()];
    };
    let Ok(entries) = fs::read_dir(prefix) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.join("Cargo.toml").is_file() {
                Some(path.display().to_string().replace('\\', "/"))
            } else {
                None
            }
        })
        .collect()
}

fn dependencies_in_section(
    value: &toml::Value,
    key: &str,
    section: &'static str,
    target: Option<String>,
    member: Option<&str>,
    manifest_path: &str,
) -> Vec<CargoDependencyImport> {
    value
        .get(key)
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|dependencies| dependencies.iter())
        .filter_map(|(alias, dependency)| {
            dependency_import(alias, dependency, section, &target, member, manifest_path)
        })
        .collect()
}

fn dependency_import(
    alias: &str,
    dependency: &toml::Value,
    section: &'static str,
    target: &Option<String>,
    member: Option<&str>,
    manifest_path: &str,
) -> Option<CargoDependencyImport> {
    if let Some(version) = dependency.as_str() {
        return Some(CargoDependencyImport {
            member: member.map(str::to_owned),
            manifest_path: manifest_path.to_owned(),
            alias: alias.to_owned(),
            package: alias.to_owned(),
            section,
            target: target.clone(),
            version: Some(version.to_owned()),
            source: Some("registry".to_owned()),
            default_features: None,
            features: Vec::new(),
        });
    }

    let table = dependency.as_table()?;
    let package = table
        .get("package")
        .and_then(toml::Value::as_str)
        .unwrap_or(alias)
        .to_owned();
    let version = table
        .get("version")
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    let source = dependency_source(table, version.as_deref());
    let default_features = table.get("default-features").and_then(toml::Value::as_bool);
    let features = table
        .get("features")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::to_owned)
        .collect();
    Some(CargoDependencyImport {
        member: member.map(str::to_owned),
        manifest_path: manifest_path.to_owned(),
        alias: alias.to_owned(),
        package,
        section,
        target: target.clone(),
        version,
        source,
        default_features,
        features,
    })
}

fn dependency_source(
    table: &toml::map::Map<String, toml::Value>,
    version: Option<&str>,
) -> Option<String> {
    table
        .get("path")
        .and_then(toml::Value::as_str)
        .map(|path| format!("path={path}"))
        .or_else(|| {
            table
                .get("git")
                .and_then(toml::Value::as_str)
                .map(|git| format!("git={git}"))
        })
        .or_else(|| version.map(|_| "registry".to_owned()))
}

fn quoted_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn split_crate_spec(crate_spec: &str) -> (String, Option<String>) {
    let Some((name, version)) = crate_spec.split_once('@') else {
        return (crate_spec.to_owned(), None);
    };
    (name.to_owned(), Some(version.to_owned()))
}

#[derive(Clone, Copy)]
enum DependencySection {
    Package,
    Workspace,
    Dev,
    Build,
    Target,
}

struct DependencySectionSelection {
    section: DependencySection,
    target: Option<String>,
}

fn target_cargo_manifest(
    manifest_path: Option<&Path>,
    member: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let root_manifest = manifest_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("Cargo.toml"));
    if let Some(member) = member {
        let root = root_manifest.parent().unwrap_or_else(|| Path::new("."));
        let member_manifest = root.join(member).join("Cargo.toml");
        if !member_manifest.is_file() {
            anyhow::bail!(
                "workspace member `{member}` does not have a Cargo.toml at {}",
                member_manifest.display()
            );
        }
        validate_workspace_member(&root_manifest, member)?;
        return Ok(member_manifest);
    }
    Ok(root_manifest)
}

fn validate_workspace_member(root_manifest: &Path, member: &str) -> anyhow::Result<()> {
    let source = fs::read_to_string(root_manifest)
        .with_context(|| format!("failed to read {}", root_manifest.display()))?;
    let parsed = source
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", root_manifest.display()))?;
    let Some(members) = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(Item::as_array)
    else {
        return Ok(());
    };
    let listed = members
        .iter()
        .filter_map(Value::as_str)
        .any(|candidate| candidate == member);
    if !listed {
        anyhow::bail!("workspace member `{member}` is not listed in workspace.members");
    }
    Ok(())
}

fn dependency_section_for_manifest(document: &DocumentMut) -> DependencySection {
    if document.get("package").is_none() && document.get("workspace").is_some() {
        DependencySection::Workspace
    } else {
        DependencySection::Package
    }
}

fn dependency_section_for_add(
    document: &DocumentMut,
    dev: bool,
    build: bool,
    target: Option<&str>,
) -> anyhow::Result<DependencySectionSelection> {
    let requested_sections = usize::from(dev) + usize::from(build) + usize::from(target.is_some());
    if requested_sections > 1 {
        anyhow::bail!(
            "K0128: choose only one dependency section flag: --dev, --build, or --target"
        );
    }
    if dev {
        return Ok(DependencySectionSelection {
            section: DependencySection::Dev,
            target: None,
        });
    }
    if build {
        return Ok(DependencySectionSelection {
            section: DependencySection::Build,
            target: None,
        });
    }
    if let Some(target) = target {
        return Ok(DependencySectionSelection {
            section: DependencySection::Target,
            target: Some(target.to_owned()),
        });
    }
    Ok(DependencySectionSelection {
        section: dependency_section_for_manifest(document),
        target: None,
    })
}

fn upsert_dependency(
    document: &mut DocumentMut,
    section: DependencySectionSelection,
    crate_name: &str,
    dependency: Item,
) -> anyhow::Result<()> {
    let dependencies = dependencies_table_mut(document, section)?;
    dependencies.insert(crate_name, dependency);
    Ok(())
}

fn dependencies_table_mut(
    document: &mut DocumentMut,
    selection: DependencySectionSelection,
) -> anyhow::Result<&mut Table> {
    match selection.section {
        DependencySection::Package => nested_table_mut(document.as_table_mut(), "dependencies"),
        DependencySection::Workspace => {
            let workspace = nested_table_mut(document.as_table_mut(), "workspace")?;
            nested_table_mut(workspace, "dependencies")
        }
        DependencySection::Dev => nested_table_mut(document.as_table_mut(), "dev-dependencies"),
        DependencySection::Build => nested_table_mut(document.as_table_mut(), "build-dependencies"),
        DependencySection::Target => {
            let target = selection
                .target
                .as_deref()
                .context("--target requires a target triple or cfg expression")?;
            let targets = nested_table_mut(document.as_table_mut(), "target")?;
            let target_table = nested_table_mut(targets, target)?;
            nested_table_mut(target_table, "dependencies")
        }
    }
}

fn nested_table_mut<'a>(table: &'a mut Table, key: &str) -> anyhow::Result<&'a mut Table> {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    table
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .with_context(|| format!("Cargo manifest key `{key}` is not a table"))
}

fn append_kobo_metadata(
    kind: MetadataKind,
    crate_name: &str,
    package: MetadataPackageSelection,
) -> anyhow::Result<()> {
    let mut output = ensure_kobo_header(read_optional("Kobo.toml")?);
    let section_name = kind.section_name();
    output.push_str(&format!("\n[[ecosystem.{section_name}]]\n"));
    output.push_str(&format!("crate = \"{crate_name}\"\n"));
    output.push_str(&format!("package = \"{}\"\n", package.package));
    if let Some(version) = package.version.as_deref() {
        output.push_str(&format!("version = \"{version}\"\n"));
    }
    output.push_str(&format!("source = \"{}\"\n", package.source));
    if let Some(registry) = package.registry.as_deref() {
        output.push_str(&format!("registry = \"{registry}\"\n"));
    }
    if let Some(checksum) = package.checksum.as_deref() {
        output.push_str(&format!("checksum = \"{checksum}\"\n"));
    }
    if let Some(compatible_crate) = package.compatible_crate.as_deref() {
        output.push_str(&format!("compatible_crate = \"{compatible_crate}\"\n"));
    }
    if let Some(metadata_path) = package.metadata_path.as_deref() {
        output.push_str(&format!(
            "metadata_path = \"{}\"\n",
            metadata_path.display().to_string().replace('\\', "/")
        ));
    }
    if let Some(declaration_path) = package.declaration_path.as_deref() {
        output.push_str(&format!("declaration_path = \"{declaration_path}\"\n"));
    }
    if let Some(declaration_hash) = package.declaration_hash.as_deref() {
        output.push_str(&format!("declaration_hash = \"{declaration_hash}\"\n"));
    }
    if let Some(adapter_runtime) = package.adapter_runtime.as_deref() {
        output.push_str(&format!("adapter_runtime = \"{adapter_runtime}\"\n"));
    }
    if let Some(capture) = package.capture.as_deref() {
        output.push_str(&format!("capture = \"{capture}\"\n"));
    }
    if let Some(trust_policy) = package.trust_policy.as_deref() {
        output.push_str(&format!("trust_policy = \"{trust_policy}\"\n"));
    }
    if let Some(signed_by) = package.signed_by.as_deref() {
        output.push_str(&format!("signed_by = \"{signed_by}\"\n"));
    }
    if package.validated {
        output.push_str("validated = true\n");
    }
    output.push_str("review_required = true\n");
    output.push_str(&format!("reason = \"optional {}\"\n", package.description));
    if let Some(summary) = package.summary.as_ref() {
        append_summary_metadata(&mut output, crate_name, summary);
    }
    fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
    eprintln!("Recorded optional Kobo {section_name} package for `{crate_name}`");
    Ok(())
}

fn append_summary_metadata(output: &mut String, crate_name: &str, summary: &RegistrySummary) {
    if summary_entry_exists(output, crate_name, &summary.hash) {
        return;
    }
    output.push_str("\n[[ecosystem.summary]]\n");
    output.push_str(&format!("crate = \"{crate_name}\"\n"));
    output.push_str(&format!("path = \"{}\"\n", summary.path));
    output.push_str(&format!("hash = \"{}\"\n", summary.hash));
}

fn summary_entry_exists(source: &str, crate_name: &str, hash: &str) -> bool {
    source.contains(&format!("crate = \"{crate_name}\""))
        && source.contains(&format!("hash = \"{hash}\""))
}

fn read_optional(path: &str) -> anyhow::Result<String> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(source),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {path}")),
    }
}

fn ensure_kobo_header(mut source: String) -> String {
    if source.trim().is_empty() {
        source.push_str("[ecosystem]\ndefault = \"opaque\"\nreplay_unknown = \"debt\"\n");
        return source;
    }
    if !source.contains("[ecosystem]") {
        if !source.ends_with('\n') {
            source.push('\n');
        }
        source.push_str("\n[ecosystem]\ndefault = \"opaque\"\nreplay_unknown = \"debt\"\n");
    }
    source
}
