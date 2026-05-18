use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

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
    let dependency = cargo_dependency_value(
        &crate_name,
        features,
        version.or(spec_version.as_deref()),
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
    append_kobo_metadata(
        "types",
        crate_name,
        &format!("kobo-types-{crate_name}"),
        "declaration metadata package",
    )
}

pub(super) fn cmd_add_adapter(crate_name: &str) -> anyhow::Result<()> {
    append_kobo_metadata(
        "adapter",
        crate_name,
        &format!("kobo-adapter-{crate_name}"),
        "simulation adapter package",
    )
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
    kind: &str,
    crate_name: &str,
    package: &str,
    description: &str,
) -> anyhow::Result<()> {
    let mut output = ensure_kobo_header(read_optional("Kobo.toml")?);
    output.push_str(&format!("\n[[ecosystem.{kind}]]\n"));
    output.push_str(&format!("crate = \"{crate_name}\"\n"));
    output.push_str(&format!("package = \"{package}\"\n"));
    output.push_str(&format!("reason = \"optional {description}\"\n"));
    fs::write("Kobo.toml", output).context("failed to write Kobo.toml")?;
    eprintln!("Recorded optional Kobo {kind} package for `{crate_name}`");
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
