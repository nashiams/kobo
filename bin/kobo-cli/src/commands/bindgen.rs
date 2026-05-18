use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use quote::ToTokens;
use syn::parse::Parser;

use super::ecosystem::{community_registry_entry, validate_registry_source_identity};

pub(super) struct BindgenOptions<'a> {
    pub(super) path: Option<&'a Path>,
    pub(super) crate_name: Option<&'a str>,
    pub(super) features: Option<&'a str>,
}

struct RegistryDraft {
    crate_name: String,
    version: String,
    features: Vec<String>,
    public_types: Vec<String>,
    public_functions: Vec<String>,
    registry: String,
}

type ImportMap = HashMap<String, Vec<String>>;

pub(super) fn cmd_bindgen(options: BindgenOptions<'_>) -> anyhow::Result<()> {
    match (options.path, options.crate_name) {
        (Some(path), None) => cmd_bindgen_path(path, options.features),
        (None, Some(crate_name)) => cmd_bindgen_registry(crate_name, options.features),
        (Some(_), Some(_)) => {
            anyhow::bail!("kobo bindgen accepts either --path or a crate name, not both")
        }
        (None, None) => anyhow::bail!("kobo bindgen needs --path <crate> or a registry crate name"),
    }
}

fn cmd_bindgen_path(path: &Path, features: Option<&str>) -> anyhow::Result<()> {
    let metadata = CargoPackageMetadata::read(path, features)?;
    let cfg = CfgEvaluation::from_features(&metadata.features);
    let sources = CrateSources::load(path, &metadata.target_src_path, &cfg)?;
    let api = PublicApi::scan(&sources, &cfg);

    eprintln!(
        "warning[K0127]: bindgen produced a source-backed declaration draft for `{}`; review effects before exact replay",
        metadata.crate_name
    );
    let mut declaration = String::new();
    declaration.push_str("schema_version = 0\n\n");
    declaration.push_str("[crate]\n");
    declaration.push_str(&format!("name = \"{}\"\n", metadata.crate_name));
    declaration.push_str(&format!("version = \"{}\"\n", metadata.version));
    declaration.push_str("source = \"bindgen\"\n");
    declaration.push_str(&format!(
        "metadata_source = \"{}\"\n",
        metadata.metadata_source
    ));
    declaration.push_str(&format!(
        "metadata_status = \"{}\"\n",
        metadata.metadata_status
    ));
    declaration.push_str("api_extraction_source = \"cargo-metadata+checked-syn-public-api\"\n");
    declaration.push_str("api_extraction_status = \"cargo-check-validated-public-api-draft\"\n");
    declaration.push_str("api_validation = \"cargo-check --lib\"\n");
    declaration.push_str(&format!(
        "manifest_path = \"{}\"\n",
        metadata
            .manifest_path
            .display()
            .to_string()
            .replace('\\', "/")
    ));
    declaration.push_str(&format!("target_kind = \"{}\"\n", metadata.target_kind));
    declaration.push_str("cfg_policy = \"conservative\"\n");
    if !metadata.features.is_empty() {
        declaration.push_str(&format!(
            "features = [{}]\n",
            quoted_list(&metadata.features)
        ));
    }
    declaration.push_str("review_required = true\n");
    declaration.push_str(&format!(
        "source_hash = \"{}\"\n",
        stable_hash(&sources.hash_material)
    ));
    declaration.push_str("declaration_hash = \"__KOBO_DECLARATION_HASH__\"\n");

    for ty in &api.types {
        declaration.push_str("\n[[type]]\n");
        declaration.push_str(&format!(
            "path = \"{}::{}\"\n",
            metadata.crate_name, ty.path
        ));
        declaration.push_str(&format!("kind = \"{}\"\n", ty.kind.as_str()));
        if !ty.generics.is_empty() {
            declaration.push_str(&format!("generics = [{}]\n", quoted_list(&ty.generics)));
        }
        if !ty.generic_bounds.is_empty() {
            declaration.push_str(&format!(
                "generic_bounds = [{}]\n",
                quoted_list(&ty.generic_bounds)
            ));
        }
        if !ty.variants.is_empty() {
            declaration.push_str(&format!("variants = [{}]\n", quoted_list(&ty.variants)));
        }
        if !ty.trait_methods.is_empty() {
            declaration.push_str(&format!(
                "trait_methods = [{}]\n",
                quoted_list(&ty.trait_methods)
            ));
        }
        if !ty.trait_associated_types.is_empty() {
            declaration.push_str(&format!(
                "trait_associated_types = [{}]\n",
                quoted_list(&ty.trait_associated_types)
            ));
        }
        if !ty.trait_associated_consts.is_empty() {
            declaration.push_str(&format!(
                "trait_associated_consts = [{}]\n",
                quoted_list(&ty.trait_associated_consts)
            ));
        }
        if let Some(alias_target) = ty.alias_target.as_deref() {
            declaration.push_str(&format!("alias_target = \"{alias_target}\"\n"));
        }
        if let Some(methods) = api.impl_methods.get(&ty.path) {
            let lifecycle = methods
                .iter()
                .filter(|method| is_lifecycle_method(method))
                .map(|method| format!("\"{method}\""))
                .collect::<Vec<_>>();
            if !lifecycle.is_empty() {
                declaration.push_str("resource = true\n");
                declaration.push_str(&format!("must_call = [{}]\n", lifecycle.join(", ")));
            }
        }
    }

    for function in &api.functions {
        declaration.push_str("\n[[function]]\n");
        declaration.push_str(&format!(
            "path = \"{}::{}\"\n",
            metadata.crate_name, function.path
        ));
        if let Some(return_type) = function.return_type.as_deref() {
            declaration.push_str(&format!(
                "returns = \"{}::{return_type}\"\n",
                metadata.crate_name
            ));
        }
        if !function.generics.is_empty() {
            declaration.push_str(&format!(
                "generics = [{}]\n",
                quoted_list(&function.generics)
            ));
        }
        if !function.generic_bounds.is_empty() {
            declaration.push_str(&format!(
                "generic_bounds = [{}]\n",
                quoted_list(&function.generic_bounds)
            ));
        }
        if !function.parameters.is_empty() {
            declaration.push_str(&format!(
                "parameters = [{}]\n",
                quoted_list(&function.parameters)
            ));
        }
        if let Some(return_type_signature) = function.return_type_signature.as_deref() {
            declaration.push_str(&format!(
                "return_type_signature = \"{return_type_signature}\"\n"
            ));
        }
        if let Some(receiver) = function.method_receiver.as_deref() {
            declaration.push_str(&format!(
                "method_receiver = \"{}::{receiver}\"\n",
                metadata.crate_name
            ));
        }
        declaration.push_str(&format!("effects = [{}]\n", quoted_list(&function.effects)));
        declaration.push_str(&format!("determinism = \"{}\"\n", function.determinism));
        declaration.push_str(&format!("replay_policy = \"{}\"\n", function.replay_policy));
        if function.replay_policy != "typed-draft" || function_needs_review(&function.path) {
            declaration.push_str(&format!(
                "review_question = \"confirm effects and replay policy for {}::{}\"\n",
                metadata.crate_name, function.path
            ));
        }
    }

    for reexport in &api.reexports {
        declaration.push_str("\n[[adapter]]\n");
        declaration.push_str(&format!(
            "path = \"{}::{}\"\n",
            metadata.crate_name, reexport.path
        ));
        declaration.push_str(&format!(
            "target = \"{}::{}\"\n",
            metadata.crate_name, reexport.target
        ));
        declaration
            .push_str("review_question = \"confirm reexported API target and ecosystem policy\"\n");
    }

    for unresolved in &api.unresolved_cfg_items {
        match &unresolved.kind {
            UnresolvedCfgKind::Type(kind) => {
                declaration.push_str("\n[[type]]\n");
                declaration.push_str(&format!(
                    "path = \"{}::{}\"\n",
                    metadata.crate_name, unresolved.name
                ));
                declaration.push_str(&format!("kind = \"{}\"\n", kind.as_str()));
            }
            UnresolvedCfgKind::Function => {
                declaration.push_str("\n[[function]]\n");
                declaration.push_str(&format!(
                    "path = \"{}::{}\"\n",
                    metadata.crate_name, unresolved.name
                ));
            }
        }
        declaration.push_str(&format!(
            "review_question = \"unresolved cfg predicate `{}`; confirm Cargo/rustc configuration before exact replay\"\n",
            unresolved.predicate
        ));
    }

    let declaration_hash = stable_hash(&declaration_without_hash(&declaration));
    print!(
        "{}",
        declaration.replace("__KOBO_DECLARATION_HASH__", &declaration_hash)
    );
    Ok(())
}

fn cmd_bindgen_registry(crate_name: &str, features: Option<&str>) -> anyhow::Result<()> {
    if let Some(source_path) = cargo_dependency_source_path(crate_name)? {
        return cmd_bindgen_path(&source_path, features);
    }
    let entry = community_registry_entry(crate_name)?.with_context(|| {
        format!(
            "K0127: no built-in v0.11 registry metadata for `{crate_name}`; use kobo bindgen --path <crate>"
        )
    })?;
    if let Some(source_path) = entry.source_path.as_deref() {
        validate_registry_source_identity(&entry)?;
        return cmd_bindgen_path(source_path, features);
    }
    let draft = RegistryDraft {
        crate_name: entry.crate_name,
        version: entry.cargo_version,
        features: feature_list(features),
        public_types: entry.public_types,
        public_functions: entry.public_functions,
        registry: entry.registry,
    };
    eprintln!(
        "warning[K0127]: bindgen produced a registry seed declaration draft for `{crate_name}`; run --path for source-backed API extraction before exact replay"
    );
    print!("{}", registry_declaration(&draft));
    Ok(())
}

fn cargo_dependency_source_path(crate_name: &str) -> anyhow::Result<Option<PathBuf>> {
    if !Path::new("Cargo.toml").is_file() {
        return Ok(None);
    }
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--offline"])
        .output()
        .context("K0127: failed to run cargo metadata for dependency bindgen")?;
    if !output.status.success() {
        return Ok(None);
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("K0127: failed to parse cargo metadata JSON for dependency bindgen")?;
    let Some(packages) = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(None);
    };
    Ok(packages
        .iter()
        .find(|package| {
            package
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| package_name_matches(name, crate_name))
        })
        .and_then(|package| {
            package
                .get("manifest_path")
                .and_then(serde_json::Value::as_str)
                .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
        }))
}

fn package_name_matches(package_name: &str, crate_name: &str) -> bool {
    package_name == crate_name || package_name.replace('-', "_") == crate_name
}

fn registry_declaration(draft: &RegistryDraft) -> String {
    let mut declaration = String::new();
    declaration.push_str("schema_version = 0\n\n");
    declaration.push_str("[crate]\n");
    declaration.push_str(&format!("name = \"{}\"\n", draft.crate_name));
    declaration.push_str(&format!("version = \"{}\"\n", draft.version));
    declaration.push_str("source = \"bindgen\"\n");
    declaration.push_str("package_source = \"registry\"\n");
    declaration.push_str("metadata_source = \"registry-seed\"\n");
    declaration.push_str("api_extraction_source = \"registry-seed\"\n");
    declaration
        .push_str("api_extraction_status = \"seed-only-use---path-for-source-backed-api\"\n");
    declaration.push_str(&format!("registry = \"{}\"\n", draft.registry));
    declaration.push_str("review_required = true\n");
    if !draft.features.is_empty() {
        declaration.push_str(&format!("features = [{}]\n", quoted_list(&draft.features)));
    }
    declaration.push_str(&format!(
        "source_hash = \"{}\"\n",
        stable_hash(&registry_hash_material(draft))
    ));
    declaration.push_str("declaration_hash = \"__KOBO_DECLARATION_HASH__\"\n");
    declaration.push_str(
        "review_question = \"registry seed only; run kobo bindgen --path <crate-source> or add the crate as a Cargo dependency for source-backed public API extraction before exact replay\"\n",
    );

    for public_type in &draft.public_types {
        declaration.push_str("\n[[type]]\n");
        declaration.push_str(&format!(
            "path = \"{}::{}\"\n",
            draft.crate_name, public_type
        ));
        declaration.push_str("kind = \"unknown\"\n");
        declaration
            .push_str("review_question = \"confirm type shape and lifecycle obligations\"\n");
    }

    for public_function in &draft.public_functions {
        declaration.push_str("\n[[function]]\n");
        declaration.push_str(&format!(
            "path = \"{}::{}\"\n",
            draft.crate_name, public_function
        ));
        declaration.push_str("effects = []\n");
        declaration.push_str("determinism = \"unknown\"\n");
        declaration.push_str("replay_policy = \"review-required\"\n");
        declaration.push_str(&format!(
            "review_question = \"confirm effects and replay policy for {}::{}\"\n",
            draft.crate_name, public_function
        ));
    }

    let declaration_hash = stable_hash(&declaration_without_hash(&declaration));
    declaration.replace("__KOBO_DECLARATION_HASH__", &declaration_hash)
}

fn feature_list(features: Option<&str>) -> Vec<String> {
    features
        .into_iter()
        .flat_map(|features| features.split(','))
        .map(str::trim)
        .filter(|feature| !feature.is_empty())
        .map(str::to_owned)
        .collect()
}

fn registry_hash_material(draft: &RegistryDraft) -> String {
    let mut material = String::new();
    material.push_str(&draft.registry);
    material.push('\n');
    material.push_str(&draft.crate_name);
    material.push('\n');
    material.push_str(&draft.version);
    material.push('\n');
    material.push_str(&draft.features.join(","));
    material.push('\n');
    material.push_str(&draft.public_types.join(","));
    material.push('\n');
    material.push_str(&draft.public_functions.join(","));
    material
}

fn declaration_without_hash(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("declaration_hash ="))
        .collect::<Vec<_>>()
        .join("\n")
}

struct CargoPackageMetadata {
    crate_name: String,
    version: String,
    manifest_path: PathBuf,
    target_src_path: PathBuf,
    target_kind: String,
    metadata_source: &'static str,
    metadata_status: &'static str,
    features: Vec<String>,
}

struct CfgEvaluation {
    enabled_features: BTreeSet<String>,
}

struct CrateSources {
    files: Vec<SourceFile>,
    hash_material: String,
}

struct SourceFile {
    module_path: Vec<String>,
    is_public_module: bool,
    path: PathBuf,
    source: String,
}

struct UnresolvedCfgItem {
    kind: UnresolvedCfgKind,
    name: String,
    predicate: String,
}

enum UnresolvedCfgKind {
    Type(PublicTypeKind),
    Function,
}

impl CargoPackageMetadata {
    fn read(crate_root: &Path, features: Option<&str>) -> anyhow::Result<Self> {
        let crate_root = crate_root
            .canonicalize()
            .unwrap_or_else(|_| crate_root.to_path_buf());
        let manifest_path = crate_root.join("Cargo.toml");
        Self::from_cargo_metadata(&crate_root, &manifest_path, features)
    }

    fn from_cargo_metadata(
        crate_root: &Path,
        manifest_path: &Path,
        features: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut command = Command::new("cargo");
        command
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline",
            ])
            .arg("--manifest-path")
            .arg(manifest_path);
        let feature_values = feature_list(features);
        if !feature_values.is_empty() {
            command.arg("--features").arg(feature_values.join(","));
        }
        let output = command.current_dir(crate_root).output();
        let output = output.with_context(|| {
            format!(
                "K0127: failed to run cargo metadata for {}",
                manifest_path.display()
            )
        })?;
        if !output.status.success() {
            anyhow::bail!(
                "K0127: cargo metadata failed for {}: {}",
                manifest_path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("failed to parse cargo metadata JSON")?;
        let Some(package) = metadata
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .and_then(|packages| packages.first())
        else {
            anyhow::bail!(
                "K0127: cargo metadata did not return package metadata for {}",
                manifest_path.display()
            );
        };
        let crate_name = package
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let version = package
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("0.0.0")
            .to_owned();
        let manifest_path = package
            .get("manifest_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| manifest_path.to_path_buf());
        let target = package
            .get("targets")
            .and_then(serde_json::Value::as_array)
            .and_then(|targets| targets.iter().find(|target| target_has_kind(target, "lib")));
        let target_kind = target
            .and_then(target_kind_label)
            .unwrap_or_else(|| "lib".to_owned());
        let target_src_path = target
            .and_then(|target| target.get("src_path"))
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| crate_root.join("src").join("lib.rs"));
        validate_cargo_checked_public_api(crate_root, &manifest_path, &feature_values)?;
        Ok(Self {
            crate_name,
            version,
            manifest_path,
            target_src_path,
            target_kind,
            metadata_source: "cargo-metadata",
            metadata_status: "resolved",
            features: feature_values,
        })
    }
}

fn validate_cargo_checked_public_api(
    crate_root: &Path,
    manifest_path: &Path,
    features: &[String],
) -> anyhow::Result<()> {
    let mut command = Command::new("cargo");
    command
        .args(["check", "--quiet", "--lib", "--offline", "--manifest-path"])
        .arg(manifest_path);
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    let output = command.current_dir(crate_root).output().with_context(|| {
        format!(
            "K0127: failed to run cargo check for {}",
            manifest_path.display()
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "K0127: cargo check failed while validating public API for {}: {}",
        manifest_path.display(),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn target_has_kind(target: &serde_json::Value, kind: &str) -> bool {
    target
        .get("kind")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|kinds| kinds.iter().any(|value| value.as_str() == Some(kind)))
}

fn target_kind_label(target: &serde_json::Value) -> Option<String> {
    target
        .get("kind")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .find(|kind| *kind == "lib")
        .map(str::to_owned)
}

impl CfgEvaluation {
    fn from_features(features: &[String]) -> Self {
        Self {
            enabled_features: features.iter().cloned().collect(),
        }
    }

    fn attr_enabled(&self, attr: &syn::Attribute) -> Option<bool> {
        if !attr.path().is_ident("cfg") {
            return Some(true);
        }
        let syn::Meta::List(list) = &attr.meta else {
            return Some(true);
        };
        self.eval_cfg_tokens(list.tokens.clone())
    }

    fn eval_cfg_tokens(&self, tokens: proc_macro2::TokenStream) -> Option<bool> {
        let predicates = parse_cfg_predicates(tokens)?;
        let mut has_unknown = false;
        for predicate in predicates {
            match self.eval_meta(&predicate) {
                Some(false) => return Some(false),
                Some(true) => {}
                None => has_unknown = true,
            }
        }
        if has_unknown {
            None
        } else {
            Some(true)
        }
    }

    fn eval_meta(&self, meta: &syn::Meta) -> Option<bool> {
        match meta {
            syn::Meta::Path(path) => self.eval_path(path),
            syn::Meta::NameValue(name_value) => self.eval_name_value(name_value),
            syn::Meta::List(list) => self.eval_list(list),
        }
    }

    fn eval_path(&self, path: &syn::Path) -> Option<bool> {
        if path.is_ident("windows") {
            return Some(cfg!(windows));
        }
        if path.is_ident("unix") {
            return Some(cfg!(unix));
        }
        if path.is_ident("debug_assertions") {
            return Some(cfg!(debug_assertions));
        }
        if path.is_ident("test") {
            return Some(false);
        }
        None
    }

    fn eval_name_value(&self, name_value: &syn::MetaNameValue) -> Option<bool> {
        if !name_value.path.is_ident("feature") {
            return None;
        }
        let syn::Expr::Lit(expr_lit) = &name_value.value else {
            return Some(false);
        };
        let syn::Lit::Str(feature) = &expr_lit.lit else {
            return Some(false);
        };
        Some(self.enabled_features.contains(&feature.value()))
    }

    fn eval_list(&self, list: &syn::MetaList) -> Option<bool> {
        if list.path.is_ident("not") {
            return parse_cfg_predicates(list.tokens.clone())
                .and_then(|predicates| {
                    predicates
                        .first()
                        .and_then(|predicate| self.eval_meta(predicate))
                })
                .map(|value| !value);
        }
        if list.path.is_ident("all") {
            let predicates = parse_cfg_predicates(list.tokens.clone())?;
            let mut has_unknown = false;
            for predicate in predicates {
                match self.eval_meta(&predicate) {
                    Some(false) => return Some(false),
                    Some(true) => {}
                    None => has_unknown = true,
                }
            }
            return (!has_unknown).then_some(true);
        }
        if list.path.is_ident("any") {
            let predicates = parse_cfg_predicates(list.tokens.clone())?;
            let mut has_unknown = false;
            for predicate in predicates {
                match self.eval_meta(&predicate) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => has_unknown = true,
                }
            }
            return (!has_unknown).then_some(false);
        }
        None
    }

    fn attrs_state(&self, attrs: &[syn::Attribute]) -> CfgState {
        let mut has_unknown = false;
        for attr in attrs {
            match self.attr_enabled(attr) {
                Some(true) => {}
                Some(false) => return CfgState::Disabled,
                None => has_unknown = true,
            }
        }
        if has_unknown {
            CfgState::Unresolved(cfg_predicates_label(attrs))
        } else {
            CfgState::Enabled
        }
    }

    fn attrs_enabled(&self, attrs: &[syn::Attribute]) -> bool {
        matches!(
            self.attrs_state(attrs),
            CfgState::Enabled | CfgState::Unresolved(_)
        )
    }
}

enum CfgState {
    Enabled,
    Disabled,
    Unresolved(String),
}

fn parse_cfg_predicates(tokens: proc_macro2::TokenStream) -> Option<Vec<syn::Meta>> {
    let parser = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
    parser
        .parse2(tokens)
        .ok()
        .map(|items| items.into_iter().collect())
}

impl CrateSources {
    fn load(crate_root: &Path, root_path: &Path, cfg: &CfgEvaluation) -> anyhow::Result<Self> {
        let mut visited = BTreeSet::new();
        let mut files = Vec::new();
        load_module(
            crate_root,
            root_path.to_path_buf(),
            Vec::new(),
            true,
            cfg,
            &mut visited,
            &mut files,
        )?;
        let mut hash_material = String::new();
        for file in &files {
            hash_material.push_str(&file.path.display().to_string());
            hash_material.push('\n');
            hash_material.push_str(if file.is_public_module {
                "public"
            } else {
                "private"
            });
            hash_material.push('\n');
            hash_material.push_str(&file.source);
            hash_material.push('\n');
        }
        Ok(Self {
            files,
            hash_material,
        })
    }
}

fn load_module(
    crate_root: &Path,
    path: PathBuf,
    module_path: Vec<String>,
    is_public_module: bool,
    cfg: &CfgEvaluation,
    visited: &mut BTreeSet<PathBuf>,
    files: &mut Vec<SourceFile>,
) -> anyhow::Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    if !visited.insert(canonical) {
        return Ok(());
    }
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let parsed = syn::parse_file(&source)
        .with_context(|| format!("failed to parse Rust source {}", path.display()))?;
    files.push(SourceFile {
        module_path: module_path.clone(),
        is_public_module,
        path: path.clone(),
        source,
    });
    let src_dir = crate_root.join("src");
    let module_dir = path.parent().unwrap_or(&src_dir);
    for item in parsed.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if !cfg.attrs_enabled(&module.attrs) {
            continue;
        }
        if module.content.is_some() {
            continue;
        }
        let name = module.ident.to_string();
        let mut next_module_path = module_path.clone();
        next_module_path.push(name.clone());
        let child_is_public = is_public_module && is_public(&module.vis);
        for candidate in [
            module_dir.join(format!("{name}.rs")),
            module_dir.join(&name).join("mod.rs"),
        ] {
            if candidate.is_file() {
                load_module(
                    crate_root,
                    candidate,
                    next_module_path,
                    child_is_public,
                    cfg,
                    visited,
                    files,
                )?;
                break;
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct PublicApi {
    types: Vec<PublicType>,
    functions: Vec<PublicFunction>,
    impl_methods: std::collections::BTreeMap<String, Vec<String>>,
    reexports: Vec<PublicReexport>,
    unresolved_cfg_items: Vec<UnresolvedCfgItem>,
}

struct PublicType {
    path: String,
    kind: PublicTypeKind,
    generics: Vec<String>,
    generic_bounds: Vec<String>,
    variants: Vec<String>,
    trait_methods: Vec<String>,
    trait_associated_types: Vec<String>,
    trait_associated_consts: Vec<String>,
    alias_target: Option<String>,
}

enum PublicTypeKind {
    Struct,
    Enum,
    Trait,
    TypeAlias,
}

impl PublicTypeKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::TypeAlias => "type_alias",
        }
    }
}

struct PublicFunction {
    path: String,
    return_type: Option<String>,
    return_type_signature: Option<String>,
    method_receiver: Option<String>,
    generics: Vec<String>,
    generic_bounds: Vec<String>,
    parameters: Vec<String>,
    effects: Vec<String>,
    determinism: &'static str,
    replay_policy: &'static str,
}

struct PublicReexport {
    path: String,
    target: String,
}

impl PublicApi {
    fn scan(sources: &CrateSources, cfg: &CfgEvaluation) -> Self {
        let mut api = Self::default();
        for source_file in &sources.files {
            let Ok(file) = syn::parse_file(&source_file.source) else {
                continue;
            };
            api.scan_file(source_file, &file, cfg);
        }
        api.types.sort_by(|left, right| left.path.cmp(&right.path));
        api.types.dedup_by(|left, right| left.path == right.path);
        api.functions
            .sort_by(|left, right| left.path.cmp(&right.path));
        api.functions
            .dedup_by(|left, right| left.path == right.path);
        api.reexports
            .sort_by(|left, right| left.path.cmp(&right.path));
        api.reexports
            .dedup_by(|left, right| left.path == right.path);
        api.unresolved_cfg_items
            .sort_by(|left, right| left.name.cmp(&right.name));
        api.unresolved_cfg_items
            .dedup_by(|left, right| left.name == right.name);
        api
    }

    fn scan_file(&mut self, source_file: &SourceFile, file: &syn::File, cfg: &CfgEvaluation) {
        self.scan_items(
            &source_file.module_path,
            source_file.is_public_module,
            &file.items,
            cfg,
        );
    }

    fn scan_items(
        &mut self,
        module_path: &[String],
        is_public_module: bool,
        items: &[syn::Item],
        cfg: &CfgEvaluation,
    ) {
        let imports = collect_scope_imports(items, cfg);
        for item in items {
            match item_cfg_state(item, cfg) {
                CfgState::Enabled => {}
                CfgState::Disabled => continue,
                CfgState::Unresolved(predicate) => {
                    self.collect_unresolved_cfg_item(
                        module_path,
                        is_public_module,
                        item,
                        predicate,
                    );
                    continue;
                }
            }
            match item {
                syn::Item::Mod(item) if item.content.is_some() => {
                    let child_is_public = is_public_module && is_public(&item.vis);
                    if !child_is_public {
                        continue;
                    }
                    let mut next_module_path = module_path.to_vec();
                    next_module_path.push(item.ident.to_string());
                    if let Some((_, items)) = &item.content {
                        self.scan_items(&next_module_path, child_is_public, items, cfg);
                    }
                }
                syn::Item::Struct(item) if is_public_module && is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::Struct,
                        generics: generic_params(&item.generics),
                        generic_bounds: generic_bounds(&item.generics),
                        variants: Vec::new(),
                        trait_methods: Vec::new(),
                        trait_associated_types: Vec::new(),
                        trait_associated_consts: Vec::new(),
                        alias_target: None,
                    });
                }
                syn::Item::Enum(item) if is_public_module && is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::Enum,
                        generics: generic_params(&item.generics),
                        generic_bounds: generic_bounds(&item.generics),
                        variants: item
                            .variants
                            .iter()
                            .map(|variant| variant.ident.to_string())
                            .collect(),
                        trait_methods: Vec::new(),
                        trait_associated_types: Vec::new(),
                        trait_associated_consts: Vec::new(),
                        alias_target: None,
                    });
                }
                syn::Item::Trait(item) if is_public_module && is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::Trait,
                        generics: generic_params(&item.generics),
                        generic_bounds: generic_bounds(&item.generics),
                        variants: Vec::new(),
                        trait_methods: item
                            .items
                            .iter()
                            .filter_map(|trait_item| {
                                let syn::TraitItem::Fn(function) = trait_item else {
                                    return None;
                                };
                                Some(function.sig.ident.to_string())
                            })
                            .collect(),
                        trait_associated_types: trait_associated_types(item),
                        trait_associated_consts: trait_associated_consts(item),
                        alias_target: None,
                    });
                }
                syn::Item::Type(item) if is_public_module && is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::TypeAlias,
                        generics: generic_params(&item.generics),
                        generic_bounds: generic_bounds(&item.generics),
                        variants: Vec::new(),
                        trait_methods: Vec::new(),
                        trait_associated_types: Vec::new(),
                        trait_associated_consts: Vec::new(),
                        alias_target: Some(item.ty.to_token_stream().to_string()),
                    });
                }
                syn::Item::Fn(item) if is_public_module && is_public(&item.vis) => {
                    let path = join_path(module_path, &item.sig.ident.to_string());
                    self.functions.push(public_function(
                        path,
                        &item.sig,
                        Some(&item.block),
                        None,
                        module_path,
                        &imports,
                    ));
                }
                syn::Item::Use(item) if is_public_module && is_public(&item.vis) => {
                    collect_reexport_paths(&item.tree, module_path, &mut self.reexports);
                }
                syn::Item::Impl(item) if is_public_module => {
                    let Some(type_name) = impl_type_ident(&item.self_ty) else {
                        continue;
                    };
                    let type_path = join_path(module_path, &type_name);
                    let methods = item
                        .items
                        .iter()
                        .filter_map(|item| {
                            let syn::ImplItem::Fn(method) = item else {
                                return None;
                            };
                            if !is_public(&method.vis) {
                                return None;
                            }
                            let method_name = method.sig.ident.to_string();
                            self.functions.push(public_function(
                                format!("{type_path}::{method_name}"),
                                &method.sig,
                                Some(&method.block),
                                Some(type_path.clone()),
                                module_path,
                                &imports,
                            ));
                            Some(method_name)
                        })
                        .collect::<Vec<_>>();
                    self.impl_methods
                        .entry(type_path)
                        .or_default()
                        .extend(methods);
                }
                _ => {}
            }
        }
    }

    fn collect_unresolved_cfg_item(
        &mut self,
        module_path: &[String],
        is_public_module: bool,
        item: &syn::Item,
        predicate: String,
    ) {
        if !is_public_module {
            return;
        }
        let Some((kind, name)) = unresolved_cfg_item(item) else {
            return;
        };
        self.unresolved_cfg_items.push(UnresolvedCfgItem {
            kind,
            name: join_path(module_path, &name),
            predicate,
        });
    }
}

fn collect_reexport_paths(
    tree: &syn::UseTree,
    module_path: &[String],
    reexports: &mut Vec<PublicReexport>,
) {
    collect_reexport_paths_with_target(tree, module_path, Vec::new(), reexports);
}

fn item_cfg_state(item: &syn::Item, cfg: &CfgEvaluation) -> CfgState {
    match item {
        syn::Item::Const(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Enum(item) => cfg.attrs_state(&item.attrs),
        syn::Item::ExternCrate(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Fn(item) => cfg.attrs_state(&item.attrs),
        syn::Item::ForeignMod(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Impl(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Macro(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Mod(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Static(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Struct(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Trait(item) => cfg.attrs_state(&item.attrs),
        syn::Item::TraitAlias(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Type(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Union(item) => cfg.attrs_state(&item.attrs),
        syn::Item::Use(item) => cfg.attrs_state(&item.attrs),
        _ => CfgState::Enabled,
    }
}

fn unresolved_cfg_item(item: &syn::Item) -> Option<(UnresolvedCfgKind, String)> {
    match item {
        syn::Item::Struct(item) if is_public(&item.vis) => Some((
            UnresolvedCfgKind::Type(PublicTypeKind::Struct),
            item.ident.to_string(),
        )),
        syn::Item::Enum(item) if is_public(&item.vis) => Some((
            UnresolvedCfgKind::Type(PublicTypeKind::Enum),
            item.ident.to_string(),
        )),
        syn::Item::Trait(item) if is_public(&item.vis) => Some((
            UnresolvedCfgKind::Type(PublicTypeKind::Trait),
            item.ident.to_string(),
        )),
        syn::Item::Type(item) if is_public(&item.vis) => Some((
            UnresolvedCfgKind::Type(PublicTypeKind::TypeAlias),
            item.ident.to_string(),
        )),
        syn::Item::Fn(item) if is_public(&item.vis) => {
            Some((UnresolvedCfgKind::Function, item.sig.ident.to_string()))
        }
        _ => None,
    }
}

fn cfg_predicates_label(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .map(|attr| attr.meta.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn collect_reexport_paths_with_target(
    tree: &syn::UseTree,
    module_path: &[String],
    prefix: Vec<String>,
    reexports: &mut Vec<PublicReexport>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_reexport_paths_with_target(&path.tree, module_path, next, reexports);
        }
        syn::UseTree::Name(name) => {
            let mut target = prefix;
            target.push(name.ident.to_string());
            reexports.push(PublicReexport {
                path: join_path(module_path, &name.ident.to_string()),
                target: target.join("::"),
            });
        }
        syn::UseTree::Rename(rename) => {
            let mut target = prefix;
            target.push(rename.ident.to_string());
            reexports.push(PublicReexport {
                path: join_path(module_path, &rename.rename.to_string()),
                target: target.join("::"),
            });
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_reexport_paths_with_target(item, module_path, prefix.clone(), reexports);
            }
        }
        syn::UseTree::Glob(_) => reexports.push(PublicReexport {
            path: join_path(module_path, "*"),
            target: prefix.join("::"),
        }),
    }
}

fn quoted_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn generic_params(generics: &syn::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .map(|param| match param {
            syn::GenericParam::Type(param) => param.ident.to_string(),
            syn::GenericParam::Lifetime(param) => param.lifetime.to_token_stream().to_string(),
            syn::GenericParam::Const(param) => param.ident.to_string(),
        })
        .collect()
}

fn generic_bounds(generics: &syn::Generics) -> Vec<String> {
    let mut bounds = generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(param) if !param.bounds.is_empty() => {
                let ident = param.ident.to_string();
                let bounds = param.bounds.to_token_stream().to_string();
                Some(format!("{ident}: {bounds}"))
            }
            syn::GenericParam::Lifetime(param) if !param.bounds.is_empty() => {
                let lifetime = param.lifetime.to_token_stream().to_string();
                let bounds = param.bounds.to_token_stream().to_string();
                Some(format!("{lifetime}: {bounds}"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(where_clause) = generics.where_clause.as_ref() {
        bounds.extend(
            where_clause
                .predicates
                .iter()
                .map(|predicate| predicate.to_token_stream().to_string()),
        );
    }
    bounds
}

fn trait_associated_types(item: &syn::ItemTrait) -> Vec<String> {
    item.items
        .iter()
        .filter_map(|trait_item| {
            let syn::TraitItem::Type(associated_type) = trait_item else {
                return None;
            };
            Some(associated_type.ident.to_string())
        })
        .collect()
}

fn trait_associated_consts(item: &syn::ItemTrait) -> Vec<String> {
    item.items
        .iter()
        .filter_map(|trait_item| {
            let syn::TraitItem::Const(associated_const) = trait_item else {
                return None;
            };
            Some(associated_const.ident.to_string())
        })
        .collect()
}

fn public_function(
    path: String,
    signature: &syn::Signature,
    body: Option<&syn::Block>,
    method_receiver: Option<String>,
    module_path: &[String],
    imports: &ImportMap,
) -> PublicFunction {
    let effects = function_effects(signature, body, imports);
    let determinism = determinism_for_effects(&effects);
    let replay_policy = replay_policy_for_function(&path, &effects);
    PublicFunction {
        path,
        return_type: return_type_ident(&signature.output).map(|name| join_path(module_path, &name)),
        return_type_signature: return_type_signature(&signature.output),
        method_receiver,
        generics: generic_params(&signature.generics),
        generic_bounds: generic_bounds(&signature.generics),
        parameters: signature_parameters(signature),
        effects,
        determinism,
        replay_policy,
    }
}

fn signature_parameters(signature: &syn::Signature) -> Vec<String> {
    signature
        .inputs
        .iter()
        .filter_map(|input| {
            let syn::FnArg::Typed(parameter) = input else {
                return None;
            };
            let name = parameter.pat.to_token_stream().to_string();
            let ty = parameter.ty.to_token_stream().to_string();
            Some(format!("{name}: {ty}"))
        })
        .collect()
}

fn return_type_signature(output: &syn::ReturnType) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    Some(ty.to_token_stream().to_string())
}

fn function_effects(
    signature: &syn::Signature,
    body: Option<&syn::Block>,
    imports: &ImportMap,
) -> Vec<String> {
    let mut material = signature.to_token_stream().to_string();
    if let Some(body) = body {
        material.push(' ');
        material.push_str(&body.to_token_stream().to_string());
    }
    let mut effects = Vec::new();
    push_effect_if(
        &mut effects,
        &material,
        "environment",
        &["std :: env", ":: env ::"],
    );
    push_effect_if(
        &mut effects,
        &material,
        "filesystem",
        &["std :: fs", "tokio :: fs"],
    );
    push_effect_if(&mut effects, &material, "process", &["std :: process"]);
    push_effect_if(
        &mut effects,
        &material,
        "network",
        &["reqwest", "hyper", "TcpStream", "UdpSocket", "http ::"],
    );
    push_effect_if(&mut effects, &material, "random", &["rand ::"]);
    push_effect_if(
        &mut effects,
        &material,
        "wall-clock",
        &["SystemTime", "Instant :: now"],
    );
    push_effect_if(&mut effects, &material, "task-spawn", &["spawn"]);
    push_effect_if(&mut effects, &material, "unsafe-ffi", &["unsafe", "extern"]);
    if let Some(body) = body {
        collect_block_effects(body, imports, &mut effects);
    }
    effects.sort();
    effects.dedup();
    effects
}

fn collect_scope_imports(items: &[syn::Item], cfg: &CfgEvaluation) -> ImportMap {
    let mut imports = HashMap::new();
    for item in items {
        if matches!(item_cfg_state(item, cfg), CfgState::Disabled) {
            continue;
        }
        let syn::Item::Use(item_use) = item else {
            continue;
        };
        collect_use_aliases(&item_use.tree, Vec::new(), &mut imports);
    }
    imports
}

fn collect_block_effects(block: &syn::Block, imports: &ImportMap, effects: &mut Vec<String>) {
    let mut scoped_imports = imports.clone();
    for statement in &block.stmts {
        match statement {
            syn::Stmt::Item(syn::Item::Use(item_use)) => {
                collect_use_aliases(&item_use.tree, Vec::new(), &mut scoped_imports);
            }
            syn::Stmt::Item(item) => collect_item_effects(item, &scoped_imports, effects),
            syn::Stmt::Local(local) => {
                if let Some(init) = &local.init {
                    collect_expr_effects(init.expr.as_ref(), &scoped_imports, effects);
                }
            }
            syn::Stmt::Expr(expr, _) => collect_expr_effects(expr, &scoped_imports, effects),
            syn::Stmt::Macro(statement_macro) => {
                let material = statement_macro.mac.to_token_stream().to_string();
                push_effect_if(effects, &material, "task-spawn", &["spawn"]);
                push_effect_if(effects, &material, "unsafe-ffi", &["unsafe", "extern"]);
            }
        }
    }
}

fn collect_item_effects(item: &syn::Item, imports: &ImportMap, effects: &mut Vec<String>) {
    match item {
        syn::Item::Fn(item_fn) => collect_block_effects(&item_fn.block, imports, effects),
        syn::Item::Mod(item_mod) => {
            if let Some((_, items)) = &item_mod.content {
                let module_imports =
                    collect_scope_imports(items, &CfgEvaluation::from_features(&[]));
                for item in items {
                    collect_item_effects(item, &module_imports, effects);
                }
            }
        }
        syn::Item::Impl(item_impl) => {
            for item in &item_impl.items {
                if let syn::ImplItem::Fn(method) = item {
                    collect_block_effects(&method.block, imports, effects);
                }
            }
        }
        _ => {}
    }
}

fn collect_expr_effects(expr: &syn::Expr, imports: &ImportMap, effects: &mut Vec<String>) {
    match expr {
        syn::Expr::Array(expr) => {
            for element in &expr.elems {
                collect_expr_effects(element, imports, effects);
            }
        }
        syn::Expr::Assign(expr) => {
            collect_expr_effects(expr.left.as_ref(), imports, effects);
            collect_expr_effects(expr.right.as_ref(), imports, effects);
        }
        syn::Expr::Async(expr) => collect_block_effects(&expr.block, imports, effects),
        syn::Expr::Await(expr) => collect_expr_effects(expr.base.as_ref(), imports, effects),
        syn::Expr::Binary(expr) => {
            collect_expr_effects(expr.left.as_ref(), imports, effects);
            collect_expr_effects(expr.right.as_ref(), imports, effects);
        }
        syn::Expr::Block(expr) => collect_block_effects(&expr.block, imports, effects),
        syn::Expr::Break(expr) => {
            if let Some(value) = expr.expr.as_ref() {
                collect_expr_effects(value.as_ref(), imports, effects);
            }
        }
        syn::Expr::Call(expr) => {
            if let syn::Expr::Path(path) = expr.func.as_ref() {
                push_effects_for_path(&path.path, imports, effects);
            } else {
                collect_expr_effects(expr.func.as_ref(), imports, effects);
            }
            for argument in &expr.args {
                collect_expr_effects(argument, imports, effects);
            }
        }
        syn::Expr::Cast(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::Closure(expr) => collect_expr_effects(expr.body.as_ref(), imports, effects),
        syn::Expr::Field(expr) => collect_expr_effects(expr.base.as_ref(), imports, effects),
        syn::Expr::ForLoop(expr) => {
            collect_expr_effects(expr.expr.as_ref(), imports, effects);
            collect_block_effects(&expr.body, imports, effects);
        }
        syn::Expr::Group(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::If(expr) => {
            collect_expr_effects(expr.cond.as_ref(), imports, effects);
            collect_block_effects(&expr.then_branch, imports, effects);
            if let Some((_, else_branch)) = expr.else_branch.as_ref() {
                collect_expr_effects(else_branch.as_ref(), imports, effects);
            }
        }
        syn::Expr::Index(expr) => {
            collect_expr_effects(expr.expr.as_ref(), imports, effects);
            collect_expr_effects(expr.index.as_ref(), imports, effects);
        }
        syn::Expr::Let(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::Loop(expr) => collect_block_effects(&expr.body, imports, effects),
        syn::Expr::Macro(expr) => {
            let material = expr.mac.to_token_stream().to_string();
            push_effect_if(effects, &material, "task-spawn", &["spawn"]);
            push_effect_if(effects, &material, "unsafe-ffi", &["unsafe", "extern"]);
        }
        syn::Expr::Match(expr) => {
            collect_expr_effects(expr.expr.as_ref(), imports, effects);
            for arm in &expr.arms {
                collect_expr_effects(arm.body.as_ref(), imports, effects);
            }
        }
        syn::Expr::MethodCall(expr) => {
            collect_expr_effects(expr.receiver.as_ref(), imports, effects);
            for argument in &expr.args {
                collect_expr_effects(argument, imports, effects);
            }
            if expr.method == "spawn" {
                effects.push("task-spawn".to_owned());
            }
        }
        syn::Expr::Paren(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::Path(expr) => push_effects_for_path(&expr.path, imports, effects),
        syn::Expr::Range(expr) => {
            if let Some(start) = expr.start.as_ref() {
                collect_expr_effects(start.as_ref(), imports, effects);
            }
            if let Some(end) = expr.end.as_ref() {
                collect_expr_effects(end.as_ref(), imports, effects);
            }
        }
        syn::Expr::Reference(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::Repeat(expr) => {
            collect_expr_effects(expr.expr.as_ref(), imports, effects);
            collect_expr_effects(expr.len.as_ref(), imports, effects);
        }
        syn::Expr::Return(expr) => {
            if let Some(value) = expr.expr.as_ref() {
                collect_expr_effects(value.as_ref(), imports, effects);
            }
        }
        syn::Expr::Struct(expr) => {
            push_effects_for_path(&expr.path, imports, effects);
            for field in &expr.fields {
                collect_expr_effects(&field.expr, imports, effects);
            }
            if let Some(rest) = expr.rest.as_ref() {
                collect_expr_effects(rest.as_ref(), imports, effects);
            }
        }
        syn::Expr::Try(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::TryBlock(expr) => collect_block_effects(&expr.block, imports, effects),
        syn::Expr::Tuple(expr) => {
            for element in &expr.elems {
                collect_expr_effects(element, imports, effects);
            }
        }
        syn::Expr::Unary(expr) => collect_expr_effects(expr.expr.as_ref(), imports, effects),
        syn::Expr::Unsafe(expr) => {
            effects.push("unsafe-ffi".to_owned());
            collect_block_effects(&expr.block, imports, effects);
        }
        syn::Expr::While(expr) => {
            collect_expr_effects(expr.cond.as_ref(), imports, effects);
            collect_block_effects(&expr.body, imports, effects);
        }
        syn::Expr::Yield(expr) => {
            if let Some(value) = expr.expr.as_ref() {
                collect_expr_effects(value.as_ref(), imports, effects);
            }
        }
        _ => {}
    }
}

fn collect_use_aliases(tree: &syn::UseTree, prefix: Vec<String>, imports: &mut ImportMap) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_aliases(&path.tree, next, imports);
        }
        syn::UseTree::Name(name) => {
            let mut full_path = prefix;
            full_path.push(name.ident.to_string());
            imports.insert(name.ident.to_string(), full_path);
        }
        syn::UseTree::Rename(rename) => {
            let mut full_path = prefix;
            full_path.push(rename.ident.to_string());
            imports.insert(rename.rename.to_string(), full_path);
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_aliases(tree, prefix.clone(), imports);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn push_effects_for_path(path: &syn::Path, imports: &ImportMap, effects: &mut Vec<String>) {
    let resolved = resolved_path_segments(path, imports);
    if path_starts_with(&resolved, &["std", "env"]) {
        effects.push("environment".to_owned());
    }
    if path_starts_with(&resolved, &["std", "fs"]) || path_starts_with(&resolved, &["tokio", "fs"])
    {
        effects.push("filesystem".to_owned());
    }
    if path_starts_with(&resolved, &["std", "process"]) {
        effects.push("process".to_owned());
    }
    if path_starts_with(&resolved, &["reqwest"])
        || path_starts_with(&resolved, &["hyper"])
        || path_starts_with(&resolved, &["http"])
        || path_ends_with_segments(&resolved, &["TcpStream"])
        || path_ends_with_segments(&resolved, &["UdpSocket"])
    {
        effects.push("network".to_owned());
    }
    if path_starts_with(&resolved, &["rand"]) || path_ends_with_segments(&resolved, &["thread_rng"])
    {
        effects.push("random".to_owned());
    }
    if path_ends_with_segments(&resolved, &["SystemTime", "now"])
        || path_ends_with_segments(&resolved, &["Instant", "now"])
    {
        effects.push("wall-clock".to_owned());
    }
    if path_ends_with_segments(&resolved, &["spawn"]) {
        effects.push("task-spawn".to_owned());
    }
}

fn resolved_path_segments(path: &syn::Path, imports: &ImportMap) -> Vec<String> {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let Some(first) = segments.first().cloned() else {
        return segments;
    };
    if let Some(imported_path) = imports.get(&first) {
        let mut resolved = imported_path.clone();
        resolved.extend(segments.into_iter().skip(1));
        return resolved;
    }
    segments
}

fn path_starts_with(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix.iter())
            .all(|(segment, expected)| segment == expected)
}

fn path_ends_with_segments(path: &[String], suffix: &[&str]) -> bool {
    path.len() >= suffix.len()
        && path[path.len() - suffix.len()..]
            .iter()
            .zip(suffix.iter())
            .all(|(segment, expected)| segment == expected)
}

fn push_effect_if(effects: &mut Vec<String>, material: &str, effect: &str, needles: &[&str]) {
    if needles.iter().any(|needle| material.contains(needle)) {
        effects.push(effect.to_owned());
    }
}

fn determinism_for_effects(effects: &[String]) -> &'static str {
    if effects.is_empty() {
        return "deterministic";
    }
    if effects.iter().any(|effect| effect == "random") {
        return "nondeterministic";
    }
    if effects.iter().any(|effect| effect == "wall-clock") {
        return "time-dependent";
    }
    if effects.iter().any(|effect| effect == "environment") {
        return "environment-dependent";
    }
    if effects.iter().any(|effect| effect == "filesystem") {
        return "filesystem-dependent";
    }
    if effects.iter().any(|effect| effect == "process") {
        return "process-dependent";
    }
    if effects.iter().any(|effect| effect == "network") {
        return "external-io-dependent";
    }
    if effects.iter().any(|effect| effect == "task-spawn") {
        return "scheduler-dependent";
    }
    if effects.iter().any(|effect| effect == "unsafe-ffi") {
        return "external-ffi-dependent";
    }
    "effect-dependent"
}

fn replay_policy_for_function(path: &str, effects: &[String]) -> &'static str {
    if effects.iter().any(|effect| effect == "network") {
        return "record-or-adapter-required";
    }
    if !effects.is_empty() || function_needs_review(path) {
        return "review-required";
    }
    "typed-draft"
}

fn join_path(module_path: &[String], leaf: &str) -> String {
    if module_path.is_empty() {
        leaf.to_owned()
    } else {
        format!("{}::{leaf}", module_path.join("::"))
    }
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn return_type_ident(output: &syn::ReturnType) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    type_ident(ty)
}

fn impl_type_ident(ty: &syn::Type) -> Option<String> {
    type_ident(ty)
}

fn type_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

fn is_lifecycle_method(method: &str) -> bool {
    matches!(
        method,
        "commit" | "rollback" | "ack" | "nack" | "reply" | "reject" | "cancel" | "close"
    )
}

fn function_needs_review(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("charge")
        || lower.contains("request")
        || lower.contains("send")
        || lower.contains("begin")
        || lower.contains("receive")
        || lower.contains("token")
}

fn stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
