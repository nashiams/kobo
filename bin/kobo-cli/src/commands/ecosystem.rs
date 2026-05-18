use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

pub(super) const BUILTIN_REGISTRY_NAME: &str = "builtin-v0.11";

#[derive(Clone, Copy)]
pub(super) struct CommunityRegistryEntry {
    pub(super) crate_name: &'static str,
    pub(super) cargo_version: &'static str,
    pub(super) types_package: Option<RegistryMetadataPackage>,
    pub(super) adapter_package: Option<RegistryMetadataPackage>,
    pub(super) public_types: &'static [&'static str],
    pub(super) public_functions: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub(super) struct RegistryMetadataPackage {
    pub(super) package: &'static str,
    pub(super) version: &'static str,
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
    registry: Option<&'static str>,
    description: &'static str,
}

pub(super) fn cmd_add(
    crate_name: &str,
    features: Option<&str>,
    version: Option<&str>,
    path: Option<&Path>,
    git: Option<&str>,
    no_default_features: bool,
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
    let registry_version = community_registry_entry(&crate_name).map(|entry| entry.cargo_version);
    let dependency = cargo_dependency_value(
        &crate_name,
        features,
        version.or(spec_version.as_deref()).or(registry_version),
        path,
        git,
        no_default_features,
    )?;
    let section = dependency_section_for_manifest(&document);
    upsert_dependency(&mut document, section, &crate_name, dependency)?;
    fs::write(&cargo_path, document.to_string())
        .with_context(|| format!("failed to write {}", cargo_path.display()))?;
    eprintln!(
        "Added Cargo dependency `{crate_name}` to {}",
        cargo_path.display()
    );
    Ok(())
}

pub(super) fn cmd_add_types(crate_name: &str) -> anyhow::Result<()> {
    let package = metadata_package_for(MetadataKind::Types, crate_name);
    append_kobo_metadata(MetadataKind::Types, crate_name, package)
}

pub(super) fn cmd_add_adapter(crate_name: &str) -> anyhow::Result<()> {
    let package = metadata_package_for(MetadataKind::Adapter, crate_name);
    append_kobo_metadata(MetadataKind::Adapter, crate_name, package)
}

pub(super) fn community_registry_entry(crate_name: &str) -> Option<CommunityRegistryEntry> {
    community_registry_entries()
        .iter()
        .copied()
        .find(|entry| entry.crate_name == crate_name)
}

pub(super) fn cmd_migrate_cargo_deps() -> anyhow::Result<()> {
    let cargo_source = fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?;
    let cargo: toml::Value = toml::from_str(&cargo_source).context("failed to parse Cargo.toml")?;
    let mut output = ensure_kobo_header(read_optional("Kobo.toml")?);
    let Some(dependencies) = cargo.get("dependencies").and_then(toml::Value::as_table) else {
        fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
        return Ok(());
    };

    for (alias, value) in dependencies {
        let package = value
            .get("package")
            .and_then(toml::Value::as_str)
            .unwrap_or(alias);
        output.push_str("\n[[ecosystem.crate]]\n");
        output.push_str(&format!("name = \"{package}\"\n"));
        if package != alias {
            output.push_str(&format!("alias = \"{alias}\"\n"));
        }
        output.push_str("policy = \"opaque\"\n");
        output
            .push_str("reason = \"imported from Cargo dependency; review before exact replay\"\n");
    }

    fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
    eprintln!("Migrated Cargo dependencies into optional Kobo ecosystem metadata");
    Ok(())
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

fn community_registry_entries() -> &'static [CommunityRegistryEntry] {
    &[
        CommunityRegistryEntry {
            crate_name: "serde_json",
            cargo_version: "1",
            types_package: Some(RegistryMetadataPackage {
                package: "kobo-types-serde-json",
                version: "1",
            }),
            adapter_package: None,
            public_types: &["Value", "Map", "Number"],
            public_functions: &["from_str", "to_string", "to_vec"],
        },
        CommunityRegistryEntry {
            crate_name: "reqwest",
            cargo_version: "0.12",
            types_package: Some(RegistryMetadataPackage {
                package: "kobo-types-reqwest",
                version: "0.12",
            }),
            adapter_package: Some(RegistryMetadataPackage {
                package: "kobo-adapter-reqwest",
                version: "0.12",
            }),
            public_types: &["Client", "RequestBuilder", "Response", "Error"],
            public_functions: &["get"],
        },
        CommunityRegistryEntry {
            crate_name: "sqlx",
            cargo_version: "0.8",
            types_package: Some(RegistryMetadataPackage {
                package: "kobo-types-sqlx",
                version: "0.8",
            }),
            adapter_package: Some(RegistryMetadataPackage {
                package: "kobo-adapter-sqlx",
                version: "0.8",
            }),
            public_types: &["Pool", "Transaction", "Executor", "Row"],
            public_functions: &["query", "query_as"],
        },
        CommunityRegistryEntry {
            crate_name: "tokio",
            cargo_version: "1",
            types_package: None,
            adapter_package: Some(RegistryMetadataPackage {
                package: "kobo-adapter-tokio",
                version: "1",
            }),
            public_types: &["task::JoinHandle", "runtime::Runtime"],
            public_functions: &["spawn", "select"],
        },
        CommunityRegistryEntry {
            crate_name: "anyhow",
            cargo_version: "1",
            types_package: Some(RegistryMetadataPackage {
                package: "kobo-types-anyhow",
                version: "1",
            }),
            adapter_package: None,
            public_types: &["Error", "Result"],
            public_functions: &["anyhow"],
        },
        CommunityRegistryEntry {
            crate_name: "clap",
            cargo_version: "4",
            types_package: Some(RegistryMetadataPackage {
                package: "kobo-types-clap",
                version: "4",
            }),
            adapter_package: None,
            public_types: &["Command", "Arg"],
            public_functions: &[],
        },
        CommunityRegistryEntry {
            crate_name: "thiserror",
            cargo_version: "1",
            types_package: Some(RegistryMetadataPackage {
                package: "kobo-types-thiserror",
                version: "1",
            }),
            adapter_package: None,
            public_types: &["Error"],
            public_functions: &[],
        },
    ]
}

fn metadata_package_for(kind: MetadataKind, crate_name: &str) -> MetadataPackageSelection {
    if let Some(entry) = community_registry_entry(crate_name) {
        let registry_package = match kind {
            MetadataKind::Types => entry.types_package,
            MetadataKind::Adapter => entry.adapter_package,
        };
        if let Some(package) = registry_package {
            return MetadataPackageSelection {
                package: package.package.to_owned(),
                version: Some(package.version.to_owned()),
                source: "registry",
                registry: Some(BUILTIN_REGISTRY_NAME),
                description: kind.description(),
            };
        }
    }
    MetadataPackageSelection {
        package: format!("{}-{crate_name}", kind.default_package_prefix()),
        version: None,
        source: "convention",
        registry: None,
        description: kind.description(),
    }
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

fn upsert_dependency(
    document: &mut DocumentMut,
    section: DependencySection,
    crate_name: &str,
    dependency: Item,
) -> anyhow::Result<()> {
    let dependencies = dependencies_table_mut(document, section)?;
    dependencies.insert(crate_name, dependency);
    Ok(())
}

fn dependencies_table_mut(
    document: &mut DocumentMut,
    section: DependencySection,
) -> anyhow::Result<&mut Table> {
    match section {
        DependencySection::Package => nested_table_mut(document.as_table_mut(), "dependencies"),
        DependencySection::Workspace => {
            let workspace = nested_table_mut(document.as_table_mut(), "workspace")?;
            nested_table_mut(workspace, "dependencies")
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
    if let Some(registry) = package.registry {
        output.push_str(&format!("registry = \"{registry}\"\n"));
    }
    output.push_str("review_required = true\n");
    output.push_str(&format!("reason = \"optional {}\"\n", package.description));
    fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
    eprintln!("Recorded optional Kobo {section_name} package for `{crate_name}`");
    Ok(())
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
