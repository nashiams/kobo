use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use quote::ToTokens;

pub(super) fn cmd_bindgen(path: &Path) -> anyhow::Result<()> {
    let manifest_path = path.join("Cargo.toml");
    let manifest_source =
        fs::read_to_string(&manifest_path).context("failed to read crate Cargo.toml")?;
    let manifest: toml::Value =
        toml::from_str(&manifest_source).context("failed to parse crate Cargo.toml")?;
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .context("Cargo.toml is missing [package]")?;
    let crate_name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .context("Cargo.toml is missing package.name")?;
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .unwrap_or("0.0.0");

    let sources = CrateSources::load(path)?;
    let api = PublicApi::scan(&sources);

    eprintln!(
        "warning[K0127]: bindgen produced a review-required declaration draft for `{crate_name}`"
    );
    let mut declaration = String::new();
    declaration.push_str("schema_version = 0\n\n");
    declaration.push_str("[crate]\n");
    declaration.push_str(&format!("name = \"{crate_name}\"\n"));
    declaration.push_str(&format!("version = \"{version}\"\n"));
    declaration.push_str("source = \"bindgen\"\n");
    declaration.push_str("review_required = true\n");
    declaration.push_str(&format!(
        "source_hash = \"{}\"\n",
        stable_hash(&sources.hash_material)
    ));
    declaration.push_str("declaration_hash = \"__KOBO_DECLARATION_HASH__\"\n");

    for ty in &api.types {
        declaration.push_str("\n[[type]]\n");
        declaration.push_str(&format!("path = \"{crate_name}::{}\"\n", ty.path));
        declaration.push_str(&format!("kind = \"{}\"\n", ty.kind.as_str()));
        if !ty.generics.is_empty() {
            declaration.push_str(&format!("generics = [{}]\n", quoted_list(&ty.generics)));
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
        declaration.push_str(&format!("path = \"{crate_name}::{}\"\n", function.path));
        if let Some(return_type) = function.return_type.as_deref() {
            declaration.push_str(&format!("returns = \"{crate_name}::{return_type}\"\n"));
        }
        if function_needs_review(&function.path) {
            declaration.push_str(&format!(
                "review_question = \"confirm effects and replay policy for {crate_name}::{}\"\n",
                function.path
            ));
        }
    }

    for reexport in &api.reexports {
        declaration.push_str("\n[[adapter]]\n");
        declaration.push_str(&format!("path = \"{crate_name}::{}\"\n", reexport.path));
        declaration.push_str(&format!("target = \"{crate_name}::{}\"\n", reexport.target));
        declaration
            .push_str("review_question = \"confirm reexported API target and ecosystem policy\"\n");
    }

    let declaration_hash = stable_hash(&declaration_without_hash(&declaration));
    print!(
        "{}",
        declaration.replace("__KOBO_DECLARATION_HASH__", &declaration_hash)
    );
    Ok(())
}

fn declaration_without_hash(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("declaration_hash ="))
        .collect::<Vec<_>>()
        .join("\n")
}

struct CrateSources {
    files: Vec<SourceFile>,
    hash_material: String,
}

struct SourceFile {
    module_path: Vec<String>,
    path: PathBuf,
    source: String,
}

impl CrateSources {
    fn load(crate_root: &Path) -> anyhow::Result<Self> {
        let mut visited = BTreeSet::new();
        let mut files = Vec::new();
        load_module(
            crate_root,
            crate_root.join("src").join("lib.rs"),
            Vec::new(),
            &mut visited,
            &mut files,
        )?;
        let mut hash_material = String::new();
        for file in &files {
            hash_material.push_str(&file.path.display().to_string());
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
        path: path.clone(),
        source,
    });
    let src_dir = crate_root.join("src");
    let module_dir = path.parent().unwrap_or(&src_dir);
    for item in parsed.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if module.content.is_some() {
            continue;
        }
        let name = module.ident.to_string();
        let mut next_module_path = module_path.clone();
        next_module_path.push(name.clone());
        for candidate in [
            module_dir.join(format!("{name}.rs")),
            module_dir.join(&name).join("mod.rs"),
        ] {
            if candidate.is_file() {
                load_module(crate_root, candidate, next_module_path, visited, files)?;
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
}

struct PublicType {
    path: String,
    kind: PublicTypeKind,
    generics: Vec<String>,
    variants: Vec<String>,
    trait_methods: Vec<String>,
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
}

struct PublicReexport {
    path: String,
    target: String,
}

impl PublicApi {
    fn scan(sources: &CrateSources) -> Self {
        let mut api = Self::default();
        for source_file in &sources.files {
            let Ok(file) = syn::parse_file(&source_file.source) else {
                continue;
            };
            api.scan_file(&source_file.module_path, &file);
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
        api
    }

    fn scan_file(&mut self, module_path: &[String], file: &syn::File) {
        for item in &file.items {
            match item {
                syn::Item::Struct(item) if is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::Struct,
                        generics: generic_params(&item.generics),
                        variants: Vec::new(),
                        trait_methods: Vec::new(),
                        alias_target: None,
                    });
                }
                syn::Item::Enum(item) if is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::Enum,
                        generics: generic_params(&item.generics),
                        variants: item
                            .variants
                            .iter()
                            .map(|variant| variant.ident.to_string())
                            .collect(),
                        trait_methods: Vec::new(),
                        alias_target: None,
                    });
                }
                syn::Item::Trait(item) if is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::Trait,
                        generics: generic_params(&item.generics),
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
                        alias_target: None,
                    });
                }
                syn::Item::Type(item) if is_public(&item.vis) => {
                    self.types.push(PublicType {
                        path: join_path(module_path, &item.ident.to_string()),
                        kind: PublicTypeKind::TypeAlias,
                        generics: generic_params(&item.generics),
                        variants: Vec::new(),
                        trait_methods: Vec::new(),
                        alias_target: Some(item.ty.to_token_stream().to_string()),
                    });
                }
                syn::Item::Fn(item) if is_public(&item.vis) => {
                    self.functions.push(PublicFunction {
                        path: join_path(module_path, &item.sig.ident.to_string()),
                        return_type: return_type_ident(&item.sig.output)
                            .map(|name| join_path(module_path, &name)),
                    });
                }
                syn::Item::Use(item) if is_public(&item.vis) => {
                    collect_reexport_paths(&item.tree, module_path, &mut self.reexports);
                }
                syn::Item::Impl(item) => {
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
                            is_public(&method.vis).then(|| method.sig.ident.to_string())
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
}

fn collect_reexport_paths(
    tree: &syn::UseTree,
    module_path: &[String],
    reexports: &mut Vec<PublicReexport>,
) {
    collect_reexport_paths_with_target(tree, module_path, Vec::new(), reexports);
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
