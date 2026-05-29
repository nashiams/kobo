use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_sim_core::backend;
use serde_json::{json, Value};
use syn::{spanned::Spanned, visit::Visit};

use super::sim_model;

const SUPPORTED_SIM_INIT_PROFILES: &[&str] = &[
    "sync",
    "async",
    "stateful-input",
    "failpoint",
    "network",
    "distributed",
];

pub(super) fn cmd_sim_init(
    target: &str,
    minimal: bool,
    profile: Option<&str>,
) -> anyhow::Result<()> {
    validate_sim_init_profile(profile)?;
    let (file, symbol) = target
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("--target must use FILE:SYMBOL"))?;
    let file = Path::new(file);
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read simulation target {}", file.display()))?;
    let Some(signature) = target_signature(&source, symbol) else {
        anyhow::bail!(
            "simulation target `{symbol}` was not found in {}",
            file.display()
        );
    };

    let document = sim_model::parse_document(source);
    let selected_profile = sim_model::target_profile(&document, symbol, profile);
    let backend = sim_model::backend_for_profile(&selected_profile);
    let artifacts = write_sim_scaffold(
        file,
        symbol,
        &selected_profile,
        backend.as_str(),
        minimal,
        &document,
        &signature,
    )?;
    let value = json!({
        "sim": {
            "target": symbol,
            "profile": selected_profile,
            "backend": backend.as_str(),
            "minimal": minimal,
            "scaffold": artifacts.scaffold_path.display().to_string(),
            "islands": [
                {
                    "name": artifacts.scenario_name,
                    "target": symbol,
                    "backend": backend.as_str(),
                    "path": artifacts.island_path.display().to_string(),
                    "source": file.display().to_string(),
                }
            ],
            "backend_imports_in_user_source": false,
        }
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn validate_sim_init_profile(profile: Option<&str>) -> anyhow::Result<()> {
    let Some(profile) = profile else {
        return Ok(());
    };
    if SUPPORTED_SIM_INIT_PROFILES.contains(&profile) {
        return Ok(());
    }
    anyhow::bail!(
        "unsupported simulation profile; expected {}",
        SUPPORTED_SIM_INIT_PROFILES.join(", ")
    )
}

struct SimArtifacts {
    scaffold_path: PathBuf,
    island_path: PathBuf,
    scenario_name: String,
}

#[derive(Clone, Debug)]
struct TargetSignature {
    is_async: bool,
    params: Vec<String>,
}

#[derive(Clone, Debug)]
struct InputFixture {
    name: String,
    type_name: String,
    expression: String,
}

fn write_sim_scaffold(
    file: &Path,
    symbol: &str,
    profile: &str,
    backend: &str,
    minimal: bool,
    document: &sim_model::ScenarioDocument,
    signature: &TargetSignature,
) -> anyhow::Result<SimArtifacts> {
    let scaffold_dir = std::env::current_dir()
        .context("failed to determine current directory")?
        .join(".kobo")
        .join("sim");
    std::fs::create_dir_all(&scaffold_dir)
        .with_context(|| format!("failed to create {}", scaffold_dir.display()))?;
    let source_path = sim_model::cli_relative_path(file)?;
    let scenario_name = format!("__kobo_sim_{}", safe_identifier(symbol));
    let island_path = scaffold_dir.join(format!("{symbol}.scenario.kobo"));
    let island_source = sim_island_source(
        &source_path,
        symbol,
        &scenario_name,
        profile,
        backend,
        signature,
    );
    std::fs::write(&island_path, island_source)
        .with_context(|| format!("failed to write {}", island_path.display()))?;

    let scaffold_path = scaffold_dir.join(format!("{symbol}.sim.json"));
    let input_fixtures = input_fixtures_for(signature);
    let scaffold = json!({
        "schema_version": 1,
        "target": {
            "source": source_path,
            "symbol": symbol,
            "async": signature.is_async,
            "params": signature.params,
        },
        "input_fixtures": input_fixtures
            .iter()
            .map(|fixture| {
                json!({
                    "name": fixture.name,
                    "type": fixture.type_name,
                    "expression": fixture.expression,
                })
            })
            .collect::<Vec<_>>(),
        "profile": profile,
        "backend": backend,
        "minimal": minimal,
        "scenario_metadata": {
            "name": scenario_name,
            "island": sim_model::cli_relative_path(&island_path)?,
            "profile": profile,
            "backend": backend,
        },
        "source_hash": document.source_hash,
        "backend_imports_in_user_source": false,
    });
    std::fs::write(&scaffold_path, serde_json::to_string_pretty(&scaffold)?)
        .with_context(|| format!("failed to write {}", scaffold_path.display()))?;
    Ok(SimArtifacts {
        scaffold_path,
        island_path,
        scenario_name,
    })
}

pub(super) fn cmd_sim_scout(
    file: &Path,
    json_output: bool,
    why: bool,
    backend_recommendations: bool,
    fix_plan: bool,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    if fix_plan {
        let session = super::session::build_session(file, None)?;
        let ecosystem_roots = EcosystemRoots::from_config(&session.config);
        return print_value(fix_plan_source(&source, file, ecosystem_roots), json_output);
    }

    if backend_recommendations {
        let recommendations = backend_recommendations_for(&source);
        print_value(
            json!({
            "backend_fit": recommendations,
            "executed": true,
            "execution_surface": "generated user Rust process adapters plus Kobo-managed semantic agreement",
            "note": "Kobo reports which adapters are executable now and which backend-native engines are reserved metadata until an adapter is linked.",
            }),
            json_output,
        )?;
        return Ok(());
    }

    let scout = if why {
        scout_why_source(&source, file)
    } else {
        scout_source(&source, file)
    };
    print_value(scout, json_output)
}

fn fix_plan_source(
    source: &str,
    file: &Path,
    ecosystem_roots: EcosystemRoots,
) -> serde_json::Value {
    let classes = ReplayClassSet::from_source(source, ecosystem_roots);
    let mut items = Vec::new();
    push_fix_plan(
        &mut items,
        &classes.wall_clock,
        "wall-clock",
        "replace direct clock reads with kobo.time.now or record a time boundary",
        "kobo.time.now",
        ["typed", "record", "activity"],
    );
    push_fix_plan(
        &mut items,
        &classes.random,
        "random",
        "replace raw random reads with a seeded scenario fixture or recorded boundary",
        "kobo.random.seeded",
        ["typed", "record", "activity"],
    );
    push_fix_plan(
        &mut items,
        &classes.environment,
        "environment",
        "move environment reads into explicit scenario input or recorded configuration",
        "kobo.config.input",
        ["typed", "record", "activity"],
    );
    push_fix_plan(
        &mut items,
        &classes.http_database,
        "http-database",
        "add a declaration or mark the side effect as record/activity before exact replay",
        "kobo bindgen --path <crate>",
        ["typed", "record", "activity", "opaque", "debt"],
    );
    push_fix_plan(
        &mut items,
        &classes.task_spawn,
        "task-spawn",
        "route task creation through kobo.task.spawn or select an explicit task boundary policy",
        "kobo.task.spawn",
        ["model", "record", "activity"],
    );
    push_fix_plan(
        &mut items,
        &classes.filesystem,
        "filesystem",
        "move file IO behind a declaration, fixture, or activity boundary",
        "kobo.fs.fixture",
        ["typed", "record", "activity", "outside"],
    );
    push_fix_plan(
        &mut items,
        &classes.socket,
        "socket",
        "move socket IO behind a declaration, loopback fixture, or activity boundary",
        "kobo.net.loopback",
        ["typed", "record", "activity", "outside", "opaque"],
    );
    push_fix_plan(
        &mut items,
        &classes.process,
        "process",
        "replace process-state reads with an explicit fixture or recorded activity",
        "kobo.process.fixture",
        ["typed", "record", "activity", "outside"],
    );
    push_fix_plan(
        &mut items,
        &classes.ffi,
        "ffi",
        "wrap FFI calls behind a declaration or keep the boundary opaque/debt until reviewed",
        "kobo bindgen --crate <ffi-wrapper>",
        ["typed", "record", "activity", "opaque", "debt"],
    );
    push_fix_plan(
        &mut items,
        &classes.observable_scheduling,
        "observable-scheduling",
        "replace observable sleeps/yields with a modeled scheduler or recorded activity",
        "kobo.scheduler.model",
        ["model", "record", "activity"],
    );

    serde_json::json!({
        "schema_version": 1,
        "target": file.display().to_string(),
        "fix_plan": items,
    })
}

#[derive(Default)]
struct ReplayClassSet {
    wall_clock: ClassEvidence,
    random: ClassEvidence,
    environment: ClassEvidence,
    http_database: ClassEvidence,
    task_spawn: ClassEvidence,
    filesystem: ClassEvidence,
    socket: ClassEvidence,
    process: ClassEvidence,
    ffi: ClassEvidence,
    observable_scheduling: ClassEvidence,
}

#[derive(Clone, Default)]
struct ClassEvidence {
    events: Vec<ClassEvent>,
}

#[derive(Clone)]
struct ClassEvent {
    call_path: Option<String>,
    span_start: usize,
    span_end: usize,
}

impl ReplayClassSet {
    fn from_source(source: &str, ecosystem_roots: EcosystemRoots) -> Self {
        let Ok(file) = syn::parse_file(source) else {
            return Self::default();
        };
        let imports = ImportIndex::from_file(&file);
        let local_roots = LocalRootIndex::from_file(&file).roots;
        let mut visitor = ReplayClassVisitor {
            source,
            imports,
            local_roots,
            scopes: Vec::new(),
            ecosystem_roots,
            classes: Self::default(),
        };
        visitor.visit_file(&file);
        visitor.classes
    }
}

#[derive(Default)]
struct ImportIndex {
    paths: BTreeMap<String, Vec<String>>,
}

impl ImportIndex {
    fn from_file(file: &syn::File) -> Self {
        let mut paths = BTreeMap::new();
        for item in &file.items {
            if let syn::Item::Use(item_use) = item {
                collect_use_tree(&item_use.tree, Vec::new(), &mut paths);
            }
        }
        Self { paths }
    }
}

#[derive(Default)]
struct LocalRootIndex {
    roots: BTreeSet<String>,
}

impl LocalRootIndex {
    fn from_file(file: &syn::File) -> Self {
        let mut roots = BTreeSet::new();
        for item in &file.items {
            if let Some(ident) = item_local_ident(item) {
                roots.insert(ident);
            }
        }
        Self { roots }
    }
}

fn collect_use_tree(
    tree: &syn::UseTree,
    prefix: Vec<String>,
    paths: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            paths.insert(path.ident.to_string(), next.clone());
            collect_use_tree(&path.tree, next, paths);
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix;
            full.push(name.ident.to_string());
            if !full.is_empty() {
                paths.insert(name.ident.to_string(), full);
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix;
            full.push(rename.ident.to_string());
            if !full.is_empty() {
                paths.insert(rename.rename.to_string(), full);
            }
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix.clone(), paths);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

struct ReplayClassVisitor<'src> {
    source: &'src str,
    imports: ImportIndex,
    local_roots: BTreeSet<String>,
    scopes: Vec<Scope>,
    ecosystem_roots: EcosystemRoots,
    classes: ReplayClassSet,
}

#[derive(Clone, Default)]
struct EcosystemRoots {
    boundary_roots: BTreeSet<String>,
}

impl EcosystemRoots {
    fn from_config(config: &kobo_driver::KoboConfig) -> Self {
        let mut boundary_roots = BTreeSet::new();
        for policy in &config.ecosystem_policy.crates {
            boundary_roots.insert(policy.name.clone());
        }
        for types in &config.ecosystem_policy.types {
            boundary_roots.insert(types.crate_name.clone());
        }
        for adapter in &config.ecosystem_policy.adapters {
            boundary_roots.insert(adapter.crate_name.clone());
        }
        for summary in &config.ecosystem_policy.summaries {
            boundary_roots.insert(summary.crate_name.clone());
        }
        for dependencies in [
            &config.dependencies,
            &config.dev_dependencies,
            &config.build_dependencies,
            &config.workspace_dependencies,
        ] {
            collect_dependency_roots(dependencies, &mut boundary_roots);
        }
        for target in &config.target_dependencies {
            collect_dependency_roots(&target.dependencies, &mut boundary_roots);
            collect_dependency_roots(&target.dev_dependencies, &mut boundary_roots);
            collect_dependency_roots(&target.build_dependencies, &mut boundary_roots);
        }
        Self { boundary_roots }
    }

    fn is_configured_boundary_root(&self, root: &str) -> bool {
        self.boundary_roots.contains(root) || known_replay_boundary_root(root)
    }
}

fn collect_dependency_roots(
    dependencies: &std::collections::HashMap<String, toml::Value>,
    roots: &mut BTreeSet<String>,
) {
    for (alias, value) in dependencies {
        roots.insert(alias.clone());
        if let toml::Value::Table(table) = value {
            if let Some(package) = table.get("package").and_then(toml::Value::as_str) {
                roots.insert(package.to_owned());
            }
        }
    }
}

#[derive(Default)]
struct Scope {
    locals: BTreeSet<String>,
    imports: BTreeMap<String, Vec<String>>,
}

enum ScopedResolution<'a> {
    Local,
    Imported(&'a [String]),
}

impl<'ast> Visit<'ast> for ReplayClassVisitor<'_> {
    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.scopes.push(Scope::default());
        if let Some(scope) = self.scopes.last_mut() {
            predeclare_block_scope(node, scope);
        }
        for statement in &node.stmts {
            self.visit_stmt(statement);
        }
        self.scopes.pop();
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        let Some((_, items)) = &node.content else {
            syn::visit::visit_item_mod(self, node);
            return;
        };

        self.scopes.push(Scope::default());
        if let Some(scope) = self.scopes.last_mut() {
            predeclare_item_slice(items, scope);
        }
        for item in items {
            self.visit_item(item);
        }
        self.scopes.pop();
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        syn::visit::visit_local(self, node);
        if let Some(scope) = self.scopes.last_mut() {
            collect_pat_locals(&node.pat, &mut scope.locals);
        }
    }

    fn visit_item_foreign_mod(&mut self, node: &'ast syn::ItemForeignMod) {
        if let Some((span_start, span_end)) = span_offsets(self.source, node.span()) {
            self.classes.ffi.push(None, span_start, span_end);
        }
        syn::visit::visit_item_foreign_mod(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let segments = path_segments(&path.path);
            if let Some((span_start, span_end)) = span_offsets(self.source, path.path.span()) {
                self.classify_path(&segments, span_start, span_end);
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();
        let Some((span_start, span_end)) = span_offsets(self.source, node.span()) else {
            syn::visit::visit_expr_method_call(self, node);
            return;
        };
        if let Some(mut resolved) = self.receiver_segments(node) {
            resolved.push(method);
            let names = resolved.iter().map(String::as_str).collect::<Vec<_>>();
            let call_path = resolved.join("::");
            if names.first() == Some(&"tokio")
                && matches!(names.last(), Some(&"spawn" | &"spawn_local"))
                && !self.is_local_root("tokio")
            {
                self.classes
                    .task_spawn
                    .push(Some(call_path.clone()), span_start, span_end);
            }
            let has_external_root = names.first().is_some_and(|root| !self.is_local_root(root));
            if has_external_root
                && (path_ends_with(&names, &["std", "thread", "sleep"])
                    || path_ends_with(&names, &["std", "thread", "yield_now"])
                    || (names.first() == Some(&"tokio")
                        && (path_ends_with(&names, &["time", "sleep"])
                            || path_ends_with(&names, &["task", "yield_now"]))))
            {
                self.classes
                    .observable_scheduling
                    .push(Some(call_path), span_start, span_end);
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

impl ReplayClassVisitor<'_> {
    fn classify_path(&mut self, segments: &[String], span_start: usize, span_end: usize) {
        let resolved = self.resolved_segments(segments);
        let names = resolved.iter().map(String::as_str).collect::<Vec<_>>();
        let call_path = if resolved.is_empty() {
            None
        } else {
            Some(resolved.join("::"))
        };
        if (path_ends_with(&names, &["std", "time", "SystemTime", "now"])
            || path_ends_with(&names, &["std", "time", "Instant", "now"]))
            && self.known_external_or_std_root(&resolved)
        {
            self.classes
                .wall_clock
                .push(call_path.clone(), span_start, span_end);
        }
        if names.first() == Some(&"rand") && !self.is_local_root("rand") {
            self.classes
                .random
                .push(call_path.clone(), span_start, span_end);
        }
        if names.first() == Some(&"std")
            && names.get(1) == Some(&"env")
            && !self.is_local_root("std")
        {
            self.classes
                .environment
                .push(call_path.clone(), span_start, span_end);
        }
        if names.first() == Some(&"std")
            && names.get(1) == Some(&"fs")
            && !self.is_local_root("std")
        {
            self.classes
                .filesystem
                .push(call_path.clone(), span_start, span_end);
        }
        if names.first() == Some(&"std")
            && names.get(1) == Some(&"net")
            && !self.is_local_root("std")
        {
            self.classes
                .socket
                .push(call_path.clone(), span_start, span_end);
        }
        if names.first() == Some(&"std")
            && names.get(1) == Some(&"process")
            && !self.is_local_root("std")
        {
            self.classes
                .process
                .push(call_path.clone(), span_start, span_end);
        }
        if names
            .first()
            .is_some_and(|name| self.ecosystem_roots.is_configured_boundary_root(name))
            && !self.is_local_root(names[0])
        {
            self.classes
                .http_database
                .push(call_path.clone(), span_start, span_end);
        }
        if names.first() == Some(&"tokio")
            && names.contains(&"spawn")
            && !self.is_local_root("tokio")
        {
            self.classes
                .task_spawn
                .push(call_path.clone(), span_start, span_end);
        }
        if path_ends_with(&names, &["std", "thread", "spawn"]) && !self.is_local_root("std") {
            self.classes
                .task_spawn
                .push(call_path.clone(), span_start, span_end);
            self.classes
                .observable_scheduling
                .push(call_path.clone(), span_start, span_end);
        }
        let has_external_root = names.first().is_some_and(|root| !self.is_local_root(root));
        if has_external_root
            && (path_ends_with(&names, &["std", "thread", "sleep"])
                || path_ends_with(&names, &["std", "thread", "yield_now"])
                || (names.first() == Some(&"tokio")
                    && (path_ends_with(&names, &["time", "sleep"])
                        || path_ends_with(&names, &["task", "yield_now"]))))
        {
            self.classes
                .observable_scheduling
                .push(call_path, span_start, span_end);
        }
    }

    fn resolved_segments(&self, segments: &[String]) -> Vec<String> {
        let Some(first) = segments.first() else {
            return Vec::new();
        };
        match self.scoped_resolution(first) {
            Some(ScopedResolution::Local) => return segments.to_vec(),
            Some(ScopedResolution::Imported(imported)) => {
                let mut resolved = imported.to_vec();
                resolved.extend(segments.iter().skip(1).cloned());
                return resolved;
            }
            None => {}
        }
        if let Some(imported) = self.imports.paths.get(first) {
            let mut resolved = imported.clone();
            resolved.extend(segments.iter().skip(1).cloned());
            return resolved;
        }
        segments.to_vec()
    }

    fn receiver_segments(&self, node: &syn::ExprMethodCall) -> Option<Vec<String>> {
        match node.receiver.as_ref() {
            syn::Expr::Path(path) => Some(self.resolved_segments(&path_segments(&path.path))),
            syn::Expr::Call(call) => {
                let syn::Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                let mut resolved = self.resolved_segments(&path_segments(&path.path));
                resolved.pop();
                Some(resolved)
            }
            _ => None,
        }
    }

    fn is_local_root(&self, root: &str) -> bool {
        match self.scoped_resolution(root) {
            Some(ScopedResolution::Local) => true,
            Some(ScopedResolution::Imported(_)) => false,
            None => self.local_roots.contains(root) && !self.imports.paths.contains_key(root),
        }
    }

    fn known_external_or_std_root(&self, resolved: &[String]) -> bool {
        let Some(root) = resolved.first() else {
            return false;
        };
        (matches!(root.as_str(), "std" | "rand" | "tokio")
            || self.ecosystem_roots.is_configured_boundary_root(root))
            && !self.is_local_root(root)
    }

    fn scoped_resolution(&self, ident: &str) -> Option<ScopedResolution<'_>> {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(ident) {
                return Some(ScopedResolution::Local);
            }
            if let Some(imported) = scope.imports.get(ident) {
                return Some(ScopedResolution::Imported(imported));
            }
        }
        None
    }
}

fn known_replay_boundary_root(root: &str) -> bool {
    matches!(
        root,
        "reqwest"
            | "sqlx"
            | "hyper"
            | "h2"
            | "tonic"
            | "postgres"
            | "tokio_postgres"
            | "mysql"
            | "redis"
            | "mongodb"
            | "diesel"
            | "sea_orm"
            | "rusqlite"
    )
}

fn predeclare_block_scope(block: &syn::Block, scope: &mut Scope) {
    for statement in &block.stmts {
        if let syn::Stmt::Item(item) = statement {
            predeclare_item(item, scope);
        }
    }
}

fn predeclare_item_slice(items: &[syn::Item], scope: &mut Scope) {
    for item in items {
        predeclare_item(item, scope);
    }
}

fn predeclare_item(item: &syn::Item, scope: &mut Scope) {
    if let Some(ident) = item_local_ident(item) {
        scope.locals.insert(ident);
    }
    if let syn::Item::Use(item_use) = item {
        collect_use_tree(&item_use.tree, Vec::new(), &mut scope.imports);
    }
}

fn item_local_ident(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Const(item) => Some(item.ident.to_string()),
        syn::Item::Enum(item) => Some(item.ident.to_string()),
        syn::Item::Fn(item) => Some(item.sig.ident.to_string()),
        syn::Item::Mod(item) => Some(item.ident.to_string()),
        syn::Item::Static(item) => Some(item.ident.to_string()),
        syn::Item::Struct(item) => Some(item.ident.to_string()),
        syn::Item::Trait(item) => Some(item.ident.to_string()),
        syn::Item::Type(item) => Some(item.ident.to_string()),
        syn::Item::Union(item) => Some(item.ident.to_string()),
        _ => None,
    }
}

fn collect_pat_locals(pat: &syn::Pat, locals: &mut BTreeSet<String>) {
    match pat {
        syn::Pat::Ident(ident) => {
            if ident.ident != "_" {
                locals.insert(ident.ident.to_string());
            }
            if let Some((_, subpat)) = &ident.subpat {
                collect_pat_locals(subpat, locals);
            }
        }
        syn::Pat::Or(pat) => {
            for case in &pat.cases {
                collect_pat_locals(case, locals);
            }
        }
        syn::Pat::Paren(pat) => collect_pat_locals(&pat.pat, locals),
        syn::Pat::Reference(pat) => collect_pat_locals(&pat.pat, locals),
        syn::Pat::Slice(pat) => {
            for elem in &pat.elems {
                collect_pat_locals(elem, locals);
            }
        }
        syn::Pat::Struct(pat) => {
            for field in &pat.fields {
                collect_pat_locals(&field.pat, locals);
            }
        }
        syn::Pat::Tuple(pat) => {
            for elem in &pat.elems {
                collect_pat_locals(elem, locals);
            }
        }
        syn::Pat::TupleStruct(pat) => {
            for elem in &pat.elems {
                collect_pat_locals(elem, locals);
            }
        }
        syn::Pat::Type(pat) => collect_pat_locals(&pat.pat, locals),
        _ => {}
    }
}

impl ClassEvidence {
    fn present(&self) -> bool {
        !self.events.is_empty()
    }

    fn push(&mut self, call_path: Option<String>, span_start: usize, span_end: usize) {
        if self.events.iter().any(|event| {
            event.call_path == call_path
                && event.span_start == span_start
                && event.span_end == span_end
        }) {
            return;
        }
        self.events.push(ClassEvent {
            call_path,
            span_start,
            span_end,
        });
    }

    fn source_spans_json(&self) -> Vec<serde_json::Value> {
        self.events
            .iter()
            .map(|event| {
                serde_json::json!({
                    "start": event.span_start,
                    "end": event.span_end,
                })
            })
            .collect()
    }

    fn first_call_path(&self) -> Option<String> {
        self.events.iter().find_map(|event| event.call_path.clone())
    }

    fn call_paths_json(&self) -> Vec<String> {
        let mut paths = self
            .events
            .iter()
            .filter_map(|event| event.call_path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn path_ends_with(path: &[&str], suffix: &[&str]) -> bool {
    path.len() >= suffix.len() && path[path.len() - suffix.len()..] == *suffix
}

fn push_fix_plan<const N: usize>(
    items: &mut Vec<serde_json::Value>,
    evidence: &ClassEvidence,
    class: &str,
    action: &str,
    first_step: &str,
    policies: [&str; N],
) {
    if evidence.present() {
        items.push(serde_json::json!({
            "class": class,
            "action": action,
            "first_step": first_step,
            "policy_options": policies.to_vec(),
            "source_spans": evidence.source_spans_json(),
            "call_paths": evidence.call_paths_json(),
            "call_path": evidence.first_call_path(),
        }));
    }
}

fn span_offsets(source: &str, span: proc_macro2::Span) -> Option<(usize, usize)> {
    let start = span.start();
    let end = span.end();
    let start_offset = line_column_offset(source, start.line, start.column)?;
    let end_offset = line_column_offset(source, end.line, end.column)?;
    if end_offset <= start_offset {
        return None;
    }
    Some((start_offset, end_offset))
}

fn line_column_offset(
    source: &str,
    one_based_line: usize,
    zero_based_column: usize,
) -> Option<usize> {
    if one_based_line == 0 {
        return None;
    }
    let line_start = source
        .split_inclusive('\n')
        .take(one_based_line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    if line_start > source.len() {
        return None;
    }
    Some((line_start + zero_based_column).min(source.len()))
}

fn target_signature(source: &str, symbol: &str) -> Option<TargetSignature> {
    source
        .lines()
        .map(str::trim)
        .find_map(|line| parse_target_signature(line, symbol))
}

fn parse_target_signature(line: &str, symbol: &str) -> Option<TargetSignature> {
    let candidates = [
        ("pub async fn ", true),
        ("async fn ", true),
        ("pub fn ", false),
        ("fn ", false),
    ];
    for (prefix, is_async) in candidates {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.strip_prefix(symbol)?;
        let rest = rest.trim_start();
        let params = rest.strip_prefix('(')?;
        let end = params.find(')')?;
        let params = params[..end]
            .split(',')
            .map(str::trim)
            .filter(|param| !param.is_empty())
            .map(str::to_owned)
            .collect();
        return Some(TargetSignature { is_async, params });
    }
    None
}

fn sim_island_source(
    source_path: &str,
    symbol: &str,
    scenario_name: &str,
    profile: &str,
    backend: &str,
    signature: &TargetSignature,
) -> String {
    let async_prefix = if signature.is_async { "async " } else { "" };
    let await_suffix = if signature.is_async { ".await" } else { "" };
    let mut source = format!(
        r#"#[kobo::scenario(profile = "{profile}")]
{async_prefix}fn {scenario_name}() {{
    // kobo: target {source_path}:{symbol}
    // kobo: backend-profile {profile}
    // kobo: backend {backend}
"#
    );
    if signature.params.is_empty() {
        source.push_str(&format!("    {symbol}(){await_suffix};\n"));
    } else if let Some(arguments) = generated_arguments(signature) {
        source.push_str(&format!("    {symbol}({arguments}){await_suffix};\n"));
    } else {
        source.push_str(&format!(
            "    // kobo: target inputs required: {}\n",
            signature.params.join(", ")
        ));
    }
    source.push_str("}\n");
    source
}

fn generated_arguments(signature: &TargetSignature) -> Option<String> {
    let fixtures = input_fixtures_for(signature);
    (fixtures.len() == signature.params.len()).then(|| {
        fixtures
            .into_iter()
            .map(|fixture| fixture.expression)
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn input_fixtures_for(signature: &TargetSignature) -> Vec<InputFixture> {
    signature
        .params
        .iter()
        .filter_map(|param| input_fixture_for_param(param))
        .collect()
}

fn input_fixture_for_param(param: &str) -> Option<InputFixture> {
    let (name, type_name) = param.split_once(':')?;
    let name = name.trim().trim_start_matches("mut ").trim().to_owned();
    let type_name = type_name.trim().to_owned();
    let expression = fixture_expression_for_type(&type_name)?;
    Some(InputFixture {
        name,
        type_name,
        expression,
    })
}

fn fixture_expression_for_type(type_name: &str) -> Option<String> {
    let compact_type = type_name.replace(' ', "");
    match compact_type.as_str() {
        "bool" => Some("false".to_owned()),
        "&str" | "&'staticstr" => Some("\"kobo-sim\"".to_owned()),
        "String" | "std::string::String" => Some("String::from(\"kobo-sim\")".to_owned()),
        "()" => Some("()".to_owned()),
        "usize" | "u8" | "u16" | "u32" | "u64" | "u128" | "isize" | "i8" | "i16" | "i32"
        | "i64" | "i128" => Some(format!("0_{compact_type}")),
        "f32" => Some("0.0_f32".to_owned()),
        "f64" => Some("0.0_f64".to_owned()),
        _ if compact_type.starts_with("Option<") => Some("None".to_owned()),
        _ if compact_type.starts_with("Vec<") => Some("Vec::new()".to_owned()),
        _ => None,
    }
}

fn safe_identifier(value: &str) -> String {
    let ident = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if ident
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        ident
    } else {
        format!("_{ident}")
    }
}

pub(super) fn cmd_sim_backends(json_output: bool) -> anyhow::Result<()> {
    let backends = backend::capabilities()
        .iter()
        .map(backend_capability_json)
        .collect::<Vec<_>>();
    print_value(
        json!({
            "executed": true,
            "execution_surface": "generated-rust-process plus Kobo-managed modeled facades",
            "reserved_metadata_registry": reserved_metadata_registry_json(),
            "backends": backends,
        }),
        json_output,
    )
}

fn backend_capability_json(capability: &backend::BackendCapability) -> Value {
    json!({
        "name": capability.name,
        "display_name": capability.display_name,
        "role": capability.role,
        "executes_now": capability.executes_now,
        "execution_status": if capability.executes_now { "executable" } else { "reserved" },
        "integration_level": capability.integration_level,
        "scenario_execution": capability.scenario_execution,
        "ecosystem_scope": capability.ecosystem_scope,
        "full_ecosystem_exploration": capability.full_ecosystem_exploration,
        "registered_boundary_exploration": capability.registered_boundary_exploration,
        "coverage_scope": capability.coverage_scope,
    })
}

fn reserved_metadata_registry_json() -> Value {
    let backends = backend::reserved_metadata_capabilities()
        .iter()
        .map(|capability| {
            json!({
                "name": capability.name,
                "display_name": capability.display_name,
                "backend_fit": capability.backend_fit,
                "metadata_only": true,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "executed": false,
        "metadata_only": true,
        "backends": backends,
    })
}

fn scout_source(source: &str, file: &Path) -> serde_json::Value {
    let target = scenario_or_function_name(source);
    let mut reasons = scout_reasons(source);

    if reasons.is_empty() {
        return json!({
            "top_target": null,
            "summary": "no high-value simulation target",
            "reasons": [],
            "next_command": null,
        });
    }

    reasons.sort();
    reasons.dedup();
    json!({
        "top_target": target.unwrap_or_else(|| "module".to_owned()),
        "score": reasons.len(),
        "reasons": reasons,
        "expected_first_value": "one liveness, nondeterminism, or boundary finding",
        "next_command": format!("kobo sim scout --json {}", file.display()),
    })
}

fn scout_why_source(source: &str, file: &Path) -> serde_json::Value {
    let recommendations = backend_recommendations_for(source);
    let scout = scout_source(source, file);

    json!({
        "kobo_posture": "Kobo is Rust-shaped and Cargo-native; backend choices are possible engines, not user source imports.",
        "source_import_policy": "normal Kobo source stays framework-shaped; backend replacement types are not default diagnostics.",
        "profile_recommendation_scope": "stable profile recommendation; generated harness execution is visible through inspect surfaces.",
        "backend_choice": "Kobo executes generated user Rust harnesses through linked scheduler, filesystem, network, and Loom adapter surfaces.",
        "backend_fit": recommendations,
        "inspect_transparency": "use kobo inspect --sim for facade and generated-harness transparency.",
        "executed": true,
        "scout": scout,
    })
}

fn scout_reasons(source: &str) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if source.contains("kobo::scenario") {
        reasons.push("scenario metadata");
    }
    if sim_model::profile_shape_for_source(source) == sim_model::TargetProfileShape::Network {
        reasons.push("network boundary");
    }
    if source.contains("tokio::spawn") || source.contains("spawn(") {
        reasons.push("async spawn boundary");
    }
    if source.contains("select!") {
        reasons.push("select cancellation boundary");
    }
    if source.contains("must_call") || contains_obligation_word(source) {
        reasons.push("liveness obligation vocabulary");
    }
    if contains_raw_nondeterminism(source) {
        reasons.push("value can change between runs");
    }
    reasons
}

fn backend_recommendations_for(source: &str) -> serde_json::Value {
    match sim_model::profile_shape_for_source(source) {
        sim_model::TargetProfileShape::Network => json!([
            backend_recommendation(
                "network-loopback",
                "generated Rust loopback network harness"
            ),
            backend_recommendation(
                "loom",
                "sync concurrency interleavings around network-facing state"
            ),
            backend_recommendation(
                "shuttle",
                "async spawn/select schedule exploration around network-facing tasks"
            ),
            backend_recommendation(
                "turmoil",
                "network schedule exploration through reserved native adapter metadata"
            ),
            backend_recommendation(
                "madsim",
                "distributed schedule exploration through reserved native adapter metadata"
            )
        ]),
        sim_model::TargetProfileShape::Async => json!([
            backend_recommendation("loom", "sync concurrency interleavings"),
            backend_recommendation("shuttle", "async spawn/select schedule exploration")
        ]),
        sim_model::TargetProfileShape::StatefulInput => json!([backend_recommendation(
            "proptest",
            "input and property exploration"
        )]),
        sim_model::TargetProfileShape::Failpoint => json!([backend_recommendation(
            "failpoints",
            "manual failure injection points"
        )]),
        sim_model::TargetProfileShape::Sync => json!([backend_recommendation(
            "loom",
            "sync concurrency interleavings"
        )]),
    }
}

fn backend_recommendation(name: &str, backend_fit: &str) -> serde_json::Value {
    let capability = backend::capabilities()
        .iter()
        .find(|capability| capability.name == name);
    json!({
        "name": capability.map(|capability| capability.display_name).unwrap_or(name),
        "backend_fit": backend_fit,
        "executes_now": capability.is_some_and(|capability| capability.executes_now),
        "execution_status": if capability.is_some_and(|capability| capability.executes_now) { "executable" } else { "reserved" },
        "integration_level": capability.map(|capability| capability.integration_level).unwrap_or("metadata-only"),
        "scenario_execution": capability.map(|capability| capability.scenario_execution).unwrap_or("unsupported-native-adapter"),
    })
}

fn scenario_or_function_name(source: &str) -> Option<String> {
    if let Some(name) = extract_scenario_name(source) {
        return Some(name);
    }

    source
        .lines()
        .map(str::trim)
        .find(|line| {
            line.starts_with("fn ")
                || line.starts_with("async fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub async fn ")
        })
        .map(super::debt::extract_fn_name_from_line)
}

fn extract_scenario_name(source: &str) -> Option<String> {
    let marker = "scenario";
    let name_marker = "name";
    for line in source.lines().map(str::trim) {
        if !line.contains(marker) || !line.contains(name_marker) {
            continue;
        }
        let name_start = line.find(name_marker)?;
        let after_name = &line[name_start + name_marker.len()..];
        let quote_start = after_name.find('"')?;
        let rest = &after_name[quote_start + 1..];
        let quote_end = rest.find('"')?;
        return Some(rest[..quote_end].to_owned());
    }
    None
}

fn contains_raw_nondeterminism(source: &str) -> bool {
    source.contains("SystemTime::now")
        || source.contains("Instant::now")
        || source.contains("rand::")
        || source.contains("thread_rng")
        || source.contains("std::fs::")
        || source.contains("std::process::")
}

fn contains_obligation_word(source: &str) -> bool {
    [
        "commit", "rollback", "ack", "nack", "reply", "cancel", "finish", "abort",
    ]
    .iter()
    .any(|word| source.contains(word))
}

fn print_value(value: serde_json::Value, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("failed to serialize scout output")?
        );
    } else {
        println!("{value}");
    }
    Ok(())
}
