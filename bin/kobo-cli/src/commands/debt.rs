use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use kobo_debt::borrow_report::{build_borrow_report, BorrowReport};
use kobo_debt::patterns::{detect_migration_patterns, format_patterns};
use kobo_debt::{build_debt_report, format_warn_early};
use kobo_driver::{lifetime_erasure_debt_report, run_kir_phase};
use kobo_ir::MustCallObligation;
use kobo_migrate::{greedy_resolve, GreedyConfig};
use syn::visit::Visit;

use super::{boundary_projection, session::build_session, summary_validation};

pub(super) fn cmd_debt(file: &Path, json: bool, summary: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    let (file_count, line_count) = count_files_and_lines(session.file_set());
    let report = build_debt_report(&kir, file_count, line_count);
    let boundary_policies = boundary_projection::projections_for_file(file, &session.config)?;

    if json {
        let mut value =
            serde_json::to_value(&report).context("failed to serialize debt report to JSON")?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "boundary_policies".to_owned(),
                serde_json::Value::Array(
                    boundary_policies
                        .iter()
                        .map(boundary_projection::BoundaryPolicyProjection::to_json_value)
                        .collect(),
                ),
            );
        }
        let json_str = serde_json::to_string_pretty(&value)
            .context("failed to serialize debt report to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    if summary {
        println!(
            "{} file(s), {} line(s) — {} RcMutShared site(s) [T1:{} T2:{} T3:{}] — {} warning(s)",
            report.file_count,
            report.line_count,
            report.inventory.rc_mut_shared,
            report.complexity.tier1,
            report.complexity.tier2,
            report.complexity.tier3,
            report.warn_early.len(),
        );
        return Ok(());
    }

    // Human-readable output.
    let human = format_warn_early(&report);
    if human.is_empty() {
        println!(
            "Debt report: {} RcMutShared site(s). No active structural warnings.",
            report.inventory.rc_mut_shared
        );
    } else {
        println!("{human}");
    }
    for boundary in &boundary_policies {
        println!("{}", boundary.debt_line());
    }

    Ok(())
}

pub(super) fn cmd_debt_cargo(root: &Path, json: bool, summary: bool) -> anyhow::Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve Cargo project at {}", root.display()))?;
    let manifest_path = root.join("Cargo.toml");
    anyhow::ensure!(
        manifest_path.is_file(),
        "kobo debt --cargo requires a Cargo.toml at {}",
        manifest_path.display()
    );

    let cargo = load_standalone_cargo_context(&root, &manifest_path)?;
    let rust_files = collect_standalone_rust_files(&root)?;
    let mut files = Vec::new();
    let mut findings = Vec::new();

    for file in rust_files {
        let source = fs::read_to_string(&file)
            .with_context(|| format!("failed to read Rust source {}", file.display()))?;
        let relative_path = relative_slash_path(&root, &file);
        files.push(StandaloneRustDebtFile {
            path: relative_path.clone(),
            line_count: source.lines().count(),
        });
        findings.extend(scan_standalone_rust_file(
            &relative_path,
            &source,
            &cargo.dependencies,
        ));
    }

    let report = StandaloneRustDebtReport {
        schema_version: 1,
        mode: "rust-cargo-standalone",
        precision: "advisory",
        blocking: false,
        cargo,
        files,
        findings,
    };

    if json {
        let json_str = serde_json::to_string_pretty(&report.to_json_value())
            .context("failed to serialize standalone Rust debt report")?;
        println!("{json_str}");
        return Ok(());
    }

    if summary {
        println!(
            "Standalone Rust debt: {} file(s), {} advisory finding(s), blocking=false",
            report.files.len(),
            report.findings.len()
        );
        return Ok(());
    }

    println!(
        "Standalone Rust debt ({}, non-blocking): package {} {}",
        report.precision, report.cargo.package, report.cargo.version
    );
    println!(
        "{} Rust file(s), {} advisory finding(s)",
        report.files.len(),
        report.findings.len()
    );
    if report.findings.is_empty() {
        println!("No standalone Rust debt candidates found.");
    } else {
        for finding in &report.findings {
            println!("{}", finding.render());
        }
    }

    Ok(())
}

fn count_files_and_lines(file_set: &kobo_ir::FileSet) -> (usize, usize) {
    let mut file_count = 0usize;
    let mut line_count = 0usize;
    for (_id, entry) in file_set.iter_files() {
        file_count += 1;
        line_count += entry.source.lines().count();
    }
    (file_count, line_count)
}

#[derive(Debug)]
struct StandaloneRustDebtReport {
    schema_version: u64,
    mode: &'static str,
    precision: &'static str,
    blocking: bool,
    cargo: StandaloneCargoContext,
    files: Vec<StandaloneRustDebtFile>,
    findings: Vec<StandaloneRustDebtFinding>,
}

impl StandaloneRustDebtReport {
    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": self.schema_version,
            "mode": self.mode,
            "precision": self.precision,
            "blocking": self.blocking,
            "cargo": self.cargo.to_json_value(),
            "files": self
                .files
                .iter()
                .map(StandaloneRustDebtFile::to_json_value)
                .collect::<Vec<_>>(),
            "findings": self
                .findings
                .iter()
                .map(StandaloneRustDebtFinding::to_json_value)
                .collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug)]
struct StandaloneCargoContext {
    root: PathBuf,
    package: String,
    version: String,
    dependencies: Vec<String>,
    metadata_source: &'static str,
}

impl StandaloneCargoContext {
    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "root": self.root.display().to_string(),
            "package": self.package,
            "version": self.version,
            "dependencies": self.dependencies,
            "metadata_source": self.metadata_source,
        })
    }
}

#[derive(Debug)]
struct StandaloneRustDebtFile {
    path: String,
    line_count: usize,
}

impl StandaloneRustDebtFile {
    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "line_count": self.line_count,
        })
    }
}

#[derive(Debug)]
struct StandaloneRustDebtFinding {
    kind: &'static str,
    category: &'static str,
    severity: &'static str,
    blocking: bool,
    file: String,
    line: usize,
    span: SourceByteSpan,
    symbol: String,
    evidence: String,
    message: String,
}

impl StandaloneRustDebtFinding {
    fn render(&self) -> String {
        format!(
            "{}:{}: {} [{}] {} ({})",
            self.file, self.line, self.kind, self.severity, self.message, self.evidence
        )
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind,
            "category": self.category,
            "severity": self.severity,
            "blocking": self.blocking,
            "file": self.file,
            "line": self.line,
            "span": {
                "start": self.span.start,
                "end": self.span.end,
            },
            "symbol": self.symbol,
            "evidence": self.evidence,
            "message": self.message,
        })
    }
}

#[derive(Copy, Clone, Debug)]
struct SourceByteSpan {
    start: usize,
    end: usize,
}

fn load_standalone_cargo_context(
    root: &Path,
    manifest_path: &Path,
) -> anyhow::Result<StandaloneCargoContext> {
    let source = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest = source
        .parse::<toml::Value>()
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let manifest_context = cargo_context_from_manifest(root, &manifest);

    match cargo_metadata_context(root) {
        Ok(mut metadata_context) => {
            if metadata_context.dependencies.is_empty() {
                metadata_context.dependencies = manifest_context.dependencies;
            }
            Ok(metadata_context)
        }
        Err(_) => Ok(manifest_context),
    }
}

fn cargo_metadata_context(root: &Path) -> anyhow::Result<StandaloneCargoContext> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .context("failed to launch cargo metadata")?;
    anyhow::ensure!(
        output.status.success(),
        "cargo metadata failed for {}",
        root.display()
    );

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse cargo metadata output")?;
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .context("cargo metadata output did not include packages")?;
    let package = metadata
        .get("root_package")
        .and_then(serde_json::Value::as_str)
        .and_then(|root_package| {
            packages.iter().find(|package| {
                package
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| id == root_package)
            })
        })
        .or_else(|| packages.first())
        .context("cargo metadata did not include a root package")?;

    let package_name = package
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let version = package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("0.0.0")
        .to_owned();
    let mut dependencies = package
        .get("dependencies")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|dependency| dependency.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    sort_and_dedup(&mut dependencies);

    Ok(StandaloneCargoContext {
        root: root.to_path_buf(),
        package: package_name,
        version,
        dependencies,
        metadata_source: "cargo metadata --no-deps",
    })
}

fn cargo_context_from_manifest(root: &Path, manifest: &toml::Value) -> StandaloneCargoContext {
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .unwrap_or("workspace")
        .to_owned();
    let version = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .unwrap_or("0.0.0")
        .to_owned();
    let mut dependencies = manifest_dependency_names(manifest);
    sort_and_dedup(&mut dependencies);

    StandaloneCargoContext {
        root: root.to_path_buf(),
        package,
        version,
        dependencies,
        metadata_source: "Cargo.toml",
    }
}

fn manifest_dependency_names(manifest: &toml::Value) -> Vec<String> {
    let mut names = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        names.extend(dependency_section_names(manifest.get(section)));
    }

    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                names.extend(dependency_section_names(target.get(section)));
            }
        }
    }

    names
}

fn dependency_section_names(section: Option<&toml::Value>) -> Vec<String> {
    section
        .and_then(toml::Value::as_table)
        .map(|dependencies| dependencies.keys().cloned().collect())
        .unwrap_or_default()
}

fn collect_standalone_rust_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_standalone_rust_files_from(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_standalone_rust_files_from(dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            if should_skip_standalone_dir(&path) {
                continue;
            }
            collect_standalone_rust_files_from(&path, files)?;
            continue;
        }
        if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn should_skip_standalone_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "target" | ".git" | ".kobo" | ".idea" | ".vscode"))
}

fn scan_standalone_rust_file(
    path: &str,
    source: &str,
    dependencies: &[String],
) -> Vec<StandaloneRustDebtFinding> {
    let ast_facts = StandaloneRustAstFacts::from_source(source);
    let mut findings = Vec::new();

    if ast_facts.references_ownership_candidates {
        collect_ownership_candidates(path, source, &mut findings);
    }
    collect_liveness_candidates(path, source, &ast_facts, &mut findings);
    collect_nondeterminism_candidates(path, source, dependencies, &mut findings);

    findings
}

fn collect_ownership_candidates(
    path: &str,
    source: &str,
    findings: &mut Vec<StandaloneRustDebtFinding>,
) {
    for (line_index, line) in source.lines().enumerate() {
        for needle in ["Rc::new", "Rc<", "RefCell::new", "RefCell<"] {
            if !line.contains(needle) {
                continue;
            }
            findings.push(standalone_finding(
                "ownership-candidate",
                "ownership-candidate",
                path,
                source,
                line_index + 1,
                needle,
                extract_let_binding(line).unwrap_or_else(|| needle.to_owned()),
                needle.to_owned(),
                "shared mutable Rust ownership may need an explicit Kobo boundary".to_owned(),
            ));
        }
    }
}

fn collect_liveness_candidates(
    path: &str,
    source: &str,
    ast_facts: &StandaloneRustAstFacts,
    findings: &mut Vec<StandaloneRustDebtFinding>,
) {
    if !ast_facts.calls_spawn {
        return;
    }

    for (line_index, line) in source.lines().enumerate() {
        let spawn_needle = if line.contains("std::thread::spawn") {
            Some("std::thread::spawn")
        } else if line.contains("tokio::spawn") {
            Some("tokio::spawn")
        } else {
            None
        };
        let Some(spawn_needle) = spawn_needle else {
            continue;
        };
        let binding = extract_let_binding(line).unwrap_or_else(|| "<unbound>".to_owned());
        if binding != "<unbound>" && spawn_handle_is_observed(source, &binding) {
            continue;
        }
        findings.push(standalone_finding(
            "liveness-candidate",
            "liveness-candidate",
            path,
            source,
            line_index + 1,
            spawn_needle,
            binding,
            spawn_needle.to_owned(),
            "spawned work handle is not visibly joined or awaited".to_owned(),
        ));
    }
}

fn collect_nondeterminism_candidates(
    path: &str,
    source: &str,
    dependencies: &[String],
    findings: &mut Vec<StandaloneRustDebtFinding>,
) {
    let mut seen = BTreeSet::new();
    for (line_index, line) in source.lines().enumerate() {
        for needle in [
            "SystemTime::now",
            "Instant::now",
            "rand::random",
            "thread_rng",
        ] {
            if line.contains(needle) && seen.insert((line_index + 1, needle.to_owned())) {
                findings.push(standalone_finding(
                    "nondeterminism-boundary-candidate",
                    "nondeterminism-boundary-candidate",
                    path,
                    source,
                    line_index + 1,
                    needle,
                    needle.to_owned(),
                    needle.to_owned(),
                    "runtime value can make replay or simulation nondeterministic".to_owned(),
                ));
            }
        }

        for dependency in dependencies {
            let crate_name = dependency.replace('-', "_");
            let needle = format!("{crate_name}::");
            if !line.contains(&needle)
                || !seen.insert((line_index + 1, format!("dependency:{crate_name}")))
            {
                continue;
            }
            findings.push(standalone_finding(
                "nondeterminism-boundary-candidate",
                "external-boundary-candidate",
                path,
                source,
                line_index + 1,
                &needle,
                dependency.clone(),
                needle.clone(),
                "third-party crate call should be reviewed as a replay boundary".to_owned(),
            ));
        }
    }
}

fn standalone_finding(
    kind: &'static str,
    category: &'static str,
    file: &str,
    source: &str,
    line: usize,
    needle: &str,
    symbol: String,
    evidence: String,
    message: String,
) -> StandaloneRustDebtFinding {
    StandaloneRustDebtFinding {
        kind,
        category,
        severity: "advisory",
        blocking: false,
        file: file.to_owned(),
        line,
        span: line_needle_span(source, line, needle),
        symbol,
        evidence,
        message,
    }
}

fn spawn_handle_is_observed(source: &str, binding: &str) -> bool {
    source.contains(&format!("{binding}.join("))
        || source.contains(&format!("{binding}.await"))
        || source.contains(&format!("{binding}.abort("))
}

fn extract_let_binding(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("let ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    let name = rest
        .split(|ch: char| ch == ':' || ch == '=' || ch.is_whitespace())
        .next()?
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.trim_start_matches('_').to_owned())
    }
}

fn line_needle_span(source: &str, target_line: usize, needle: &str) -> SourceByteSpan {
    let mut offset = 0usize;
    for (line_index, line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        if line_number == target_line {
            let column = line.find(needle).unwrap_or(0);
            let start = offset + column;
            return SourceByteSpan {
                start,
                end: start + needle.len(),
            };
        }
        offset += line.len() + 1;
    }
    SourceByteSpan { start: 0, end: 0 }
}

#[derive(Default)]
struct StandaloneRustAstFacts {
    references_ownership_candidates: bool,
    calls_spawn: bool,
}

impl StandaloneRustAstFacts {
    fn from_source(source: &str) -> Self {
        let Ok(file) = syn::parse_file(source) else {
            return Self {
                references_ownership_candidates: source.contains("Rc")
                    || source.contains("RefCell"),
                calls_spawn: source.contains("spawn("),
            };
        };
        let mut facts = Self::default();
        facts.visit_file(&file);
        facts
    }
}

impl<'ast> Visit<'ast> for StandaloneRustAstFacts {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let segments = path_segments(&path.path);
            if segments.as_slice() == ["std", "thread", "spawn"]
                || segments.as_slice() == ["tokio", "spawn"]
            {
                self.calls_spawn = true;
            }
            if segments
                .iter()
                .any(|segment| segment == "Rc" || segment == "RefCell")
            {
                self.references_ownership_candidates = true;
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if node
            .path
            .segments
            .iter()
            .any(|segment| segment.ident == "Rc" || segment.ident == "RefCell")
        {
            self.references_ownership_candidates = true;
        }
        syn::visit::visit_type_path(self, node);
    }
}

fn relative_slash_path(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sort_and_dedup(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

pub(super) fn cmd_debt_liveness(file: &Path, json: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    let mut findings = build_liveness_findings(&source, kir.must_call_obligations());
    findings.extend(summary_liveness_findings(&session.config, &source)?);
    if json {
        let values = findings
            .iter()
            .map(LivenessFinding::to_json_value)
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&values)
                .context("failed to serialize liveness debt report")?
        );
        return Ok(());
    }

    if findings.is_empty() {
        println!("Liveness debt: no unresolved must_call obligations.");
        return Ok(());
    }

    for finding in findings {
        println!("{}", finding.render());
    }
    Ok(())
}

fn summary_liveness_findings(
    config: &kobo_driver::KoboConfig,
    source: &str,
) -> anyhow::Result<Vec<LivenessFinding>> {
    let mut findings = Vec::new();
    for summary in &config.ecosystem_policy.summaries {
        let valid = summary_validation::load_valid_summary(summary)?;
        let parsed = valid.value;
        let Some(obligations) = parsed
            .get("obligations")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for obligation in obligations {
            let owner_type = obligation
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("external obligation")
                .to_owned();
            let actions = obligation
                .get("terminal_actions")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let applicable_types = string_array_field(obligation, "applicable_types")
                .filter(|types| !types.is_empty())
                .unwrap_or_else(|| vec![owner_type.clone()]);
            let applicable_functions = string_array_field(obligation, "applicable_functions")
                .filter(|functions| !functions.is_empty())
                .unwrap_or_else(|| vec![summary.crate_name.clone()]);
            let applicability_source = obligation
                .get("applicability")
                .and_then(serde_json::Value::as_object)
                .and_then(|applicability| applicability.get("source"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    obligation
                        .get("applicability_source")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or("summary-declared")
                .to_owned();
            if !summary_obligation_applies(source, &applicable_types, &applicable_functions) {
                continue;
            }
            findings.push(LivenessFinding {
                code: "K0126",
                owner_type,
                actions,
                function: applicable_functions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| summary.crate_name.clone()),
                binding: None,
                applicable_types,
                applicable_functions,
                applicability_source,
                message: "liveness fact imported from .kobo-summary".to_owned(),
                reason: Some(format!(
                    "summary {} schema 1 hash {}",
                    summary.path.display(),
                    valid.hash
                )),
            });
        }
    }
    Ok(findings)
}

fn summary_obligation_applies(
    source: &str,
    applicable_types: &[String],
    applicable_functions: &[String],
) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    let applicability = SummaryApplicability::from_file(&file);
    applicable_types
        .iter()
        .any(|type_name| applicability.references_type(type_name))
        || applicable_functions
            .iter()
            .any(|function| applicability.references_function(function))
}

struct SummaryApplicability {
    local_types: BTreeSet<String>,
    imports: BTreeMap<String, Vec<String>>,
    referenced_paths: Vec<Vec<String>>,
    call_paths: Vec<Vec<String>>,
}

impl SummaryApplicability {
    fn from_file(file: &syn::File) -> Self {
        let mut applicability = Self {
            local_types: local_type_names(file),
            imports: import_paths(file),
            referenced_paths: Vec::new(),
            call_paths: Vec::new(),
        };
        applicability.visit_file(file);
        applicability
    }

    fn references_type(&self, expected: &str) -> bool {
        let expected_segments = split_path(expected);
        self.referenced_paths
            .iter()
            .any(|path| self.type_path_matches(path, &expected_segments))
    }

    fn references_function(&self, expected: &str) -> bool {
        let expected_segments = split_path(expected);
        self.call_paths
            .iter()
            .any(|path| self.path_matches(path, &expected_segments))
    }

    fn path_matches(&self, path: &[String], expected: &[String]) -> bool {
        if expected.is_empty() {
            return false;
        }
        self.resolved_candidates(path).into_iter().any(|candidate| {
            candidate == expected || (expected.len() == 1 && candidate.last() == expected.first())
        })
    }

    fn type_path_matches(&self, path: &[String], expected: &[String]) -> bool {
        if expected.is_empty() {
            return false;
        }
        self.resolved_candidates(path).into_iter().any(|candidate| {
            if candidate == expected {
                return !(expected.len() == 1
                    && candidate.len() == 1
                    && self.local_types.contains(&expected[0]));
            }
            expected.len() == 1
                && candidate.last() == expected.first()
                && !(candidate.len() == 1 && self.local_types.contains(&expected[0]))
        })
    }

    fn resolved_candidates(&self, path: &[String]) -> Vec<Vec<String>> {
        let mut candidates = vec![path.to_vec()];
        if let Some(first) = path.first() {
            if let Some(imported) = self.imports.get(first) {
                let mut resolved = imported.clone();
                resolved.extend(path.iter().skip(1).cloned());
                candidates.push(resolved);
            }
        }
        candidates
    }
}

impl<'ast> Visit<'ast> for SummaryApplicability {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            self.call_paths.push(path_segments(&path.path));
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        self.referenced_paths.push(path_segments(&node.path));
        syn::visit::visit_expr_struct(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        self.referenced_paths.push(path_segments(&node.path));
        syn::visit::visit_type_path(self, node);
    }
}

fn local_type_names(file: &syn::File) -> BTreeSet<String> {
    let mut collector = LocalTypeCollector::default();
    collector.visit_file(file);
    collector.names
}

#[derive(Default)]
struct LocalTypeCollector {
    names: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for LocalTypeCollector {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.names.insert(item.ident.to_string());
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.names.insert(item.ident.to_string());
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.names.insert(item.ident.to_string());
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.names.insert(item.ident.to_string());
        syn::visit::visit_item_trait(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.names.insert(item.ident.to_string());
        syn::visit::visit_item_union(self, item);
    }
}

fn import_paths(file: &syn::File) -> BTreeMap<String, Vec<String>> {
    let mut imports = BTreeMap::new();
    for item in &file.items {
        if let syn::Item::Use(item_use) = item {
            collect_use_tree(&item_use.tree, Vec::new(), &mut imports);
        }
    }
    imports
}

fn collect_use_tree(
    tree: &syn::UseTree,
    prefix: Vec<String>,
    imports: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_tree(&path.tree, next, imports);
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix;
            full.push(name.ident.to_string());
            imports.insert(name.ident.to_string(), full);
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix;
            full.push(rename.ident.to_string());
            imports.insert(rename.rename.to_string(), full);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix.clone(), imports);
            }
        }
        syn::UseTree::Glob(_) => {}
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

fn string_array_field(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
}

#[derive(Debug)]
struct LivenessFinding {
    code: &'static str,
    owner_type: String,
    actions: Vec<String>,
    function: String,
    binding: Option<String>,
    applicable_types: Vec<String>,
    applicable_functions: Vec<String>,
    applicability_source: String,
    message: String,
    reason: Option<String>,
}

impl LivenessFinding {
    fn render(&self) -> String {
        let action_list = self.actions.join(" | ");
        let binding = self
            .binding
            .as_ref()
            .map(|name| format!(" `{name}`"))
            .unwrap_or_default();
        let mut line = format!(
            "warning[{}]: {}{} in `{}`: {} ({})",
            self.code, self.owner_type, binding, self.function, self.message, action_list
        );
        line.push_str(&format!(
            "; applicability: source={} types=[{}] functions=[{}]",
            self.applicability_source,
            self.applicable_types.join(", "),
            self.applicable_functions.join(", ")
        ));
        if let Some(reason) = &self.reason {
            line.push_str(&format!("; reason: {reason}"));
        }
        line
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "owner_type": self.owner_type,
            "actions": self.actions,
            "function": self.function,
            "binding": self.binding,
            "applicability": {
                "source": self.applicability_source,
                "types": self.applicable_types,
                "functions": self.applicable_functions,
            },
            "message": self.message,
            "reason": self.reason,
        })
    }
}

#[derive(Debug)]
struct SourceFunction {
    name: String,
    header: String,
    body: String,
    attrs: Vec<String>,
}

#[derive(Debug)]
struct Suppression {
    has_k0100: bool,
    reason: Option<String>,
}

fn build_liveness_findings(
    source: &str,
    obligations: &[MustCallObligation],
) -> Vec<LivenessFinding> {
    let functions = collect_source_functions(source);
    let mut findings = Vec::new();

    for obligation in obligations {
        let actions = obligation
            .actions
            .iter()
            .map(|action| action.name.clone())
            .collect::<Vec<_>>();
        for function in &functions {
            let suppression = parse_suppression(&function.attrs);
            if let Some(finding) =
                escape_finding(function, obligation, &actions, suppression.as_ref())
            {
                findings.push(finding);
                continue;
            }

            for binding in declared_obligation_bindings(function, obligation) {
                match suppression.as_ref() {
                    Some(Suppression {
                        has_k0100: true,
                        reason: Some(reason),
                    }) => {
                        findings.push(LivenessFinding {
                            code: "K0108",
                            owner_type: obligation.owner_type.clone(),
                            actions: actions.clone(),
                            function: function.name.clone(),
                            binding: Some(binding),
                            applicable_types: vec![obligation.owner_type.clone()],
                            applicable_functions: vec![function.name.clone()],
                            applicability_source: "local-declaration".to_owned(),
                            message: "must_call liveness obligation suppressed".to_owned(),
                            reason: Some(reason.clone()),
                        });
                    }
                    Some(Suppression {
                        has_k0100: true,
                        reason: None,
                    }) => {
                        findings.push(LivenessFinding {
                            code: "K0100",
                            owner_type: obligation.owner_type.clone(),
                            actions: actions.clone(),
                            function: function.name.clone(),
                            binding: Some(binding),
                            applicable_types: vec![obligation.owner_type.clone()],
                            applicable_functions: vec![function.name.clone()],
                            applicability_source: "local-declaration".to_owned(),
                            message: "`#[kobo::suppress(K0100)]` requires a reason".to_owned(),
                            reason: None,
                        });
                    }
                    _ if has_unresolved_exit(&function.body, &binding, &actions) => {
                        findings.push(LivenessFinding {
                            code: "K0100",
                            owner_type: obligation.owner_type.clone(),
                            actions: actions.clone(),
                            function: function.name.clone(),
                            binding: Some(binding),
                            applicable_types: vec![obligation.owner_type.clone()],
                            applicable_functions: vec![function.name.clone()],
                            applicability_source: "local-declaration".to_owned(),
                            message: "may leave without a required call".to_owned(),
                            reason: None,
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    findings
}

fn escape_finding(
    function: &SourceFunction,
    obligation: &MustCallObligation,
    actions: &[String],
    suppression: Option<&Suppression>,
) -> Option<LivenessFinding> {
    if suppression.is_some_and(|suppression| suppression.has_k0100) {
        return None;
    }

    if !function
        .header
        .contains(&format!("-> {}", obligation.owner_type))
    {
        return None;
    }

    if !function
        .body
        .contains(&format!("{} {{", obligation.owner_type))
    {
        return None;
    }

    Some(LivenessFinding {
        code: "K0101",
        owner_type: obligation.owner_type.clone(),
        actions: actions.to_vec(),
        function: function.name.clone(),
        binding: None,
        applicable_types: vec![obligation.owner_type.clone()],
        applicable_functions: vec![function.name.clone()],
        applicability_source: "local-declaration".to_owned(),
        message: "obligation escapes local analysis through a return value".to_owned(),
        reason: None,
    })
}

fn declared_obligation_bindings(
    function: &SourceFunction,
    obligation: &MustCallObligation,
) -> Vec<String> {
    function
        .body
        .lines()
        .filter_map(|line| declared_binding_on_line(line, &obligation.owner_type))
        .collect()
}

fn declared_binding_on_line(line: &str, owner_type: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("let ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    let name = rest
        .split(|ch: char| ch == ':' || ch == '=' || ch.is_whitespace())
        .next()?
        .trim()
        .to_owned();
    if name.is_empty() {
        return None;
    }

    let typed = rest.contains(&format!(": {owner_type}"));
    let constructed = rest.contains(&format!("{owner_type} {{"));
    if typed || constructed {
        Some(name)
    } else {
        None
    }
}

fn has_unresolved_exit(body: &str, binding: &str, actions: &[String]) -> bool {
    let mut seen_binding = false;
    let mut seen_action = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&format!("let {binding} "))
            || trimmed.starts_with(&format!("let {binding} ="))
            || trimmed.starts_with(&format!("let mut {binding} "))
            || trimmed.starts_with(&format!("let mut {binding} ="))
        {
            seen_binding = true;
        }

        if !seen_binding {
            continue;
        }

        if actions
            .iter()
            .any(|action| trimmed.contains(&format!("{binding}.{action}(")))
        {
            seen_action = true;
        }

        if trimmed.starts_with("return") && !seen_action {
            return true;
        }
    }

    !seen_action
}

fn parse_suppression(attrs: &[String]) -> Option<Suppression> {
    let attr = attrs
        .iter()
        .find(|attr| attr.contains("kobo::suppress") && attr.contains("K0100"))?;
    Some(Suppression {
        has_k0100: true,
        reason: extract_reason(attr),
    })
}

fn extract_reason(attr: &str) -> Option<String> {
    let marker = "reason";
    let start = attr.find(marker)?;
    let after_marker = &attr[start + marker.len()..];
    let quote_start = after_marker.find('"')?;
    let rest = &after_marker[quote_start + 1..];
    let quote_end = rest.find('"')?;
    Some(rest[..quote_end].to_owned())
}

fn collect_source_functions(source: &str) -> Vec<SourceFunction> {
    let mut functions = Vec::new();
    let lines = source.lines().collect::<Vec<_>>();
    let mut pending_attrs = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with("#[") {
            pending_attrs.push(trimmed.to_owned());
            index += 1;
            continue;
        }

        if is_function_header(trimmed) {
            let attrs = std::mem::take(&mut pending_attrs);
            let (function, next_index) = collect_function(&lines, index, attrs);
            functions.push(function);
            index = next_index;
            continue;
        }

        if !trimmed.is_empty() {
            pending_attrs.clear();
        }
        index += 1;
    }

    functions
}

fn collect_function(lines: &[&str], start: usize, attrs: Vec<String>) -> (SourceFunction, usize) {
    let mut body_lines = Vec::new();
    let header = lines[start].trim().to_owned();
    let name = extract_fn_name_from_line(&header);
    let mut brace_depth = 0i32;
    let mut index = start;

    while index < lines.len() {
        let line = lines[index];
        brace_depth += line.matches('{').count() as i32;
        brace_depth -= line.matches('}').count() as i32;
        body_lines.push(line);
        index += 1;
        if brace_depth <= 0 && body_lines.iter().any(|line| line.contains('{')) {
            break;
        }
    }

    (
        SourceFunction {
            name,
            header,
            body: body_lines.join("\n"),
            attrs,
        },
        index,
    )
}

fn is_function_header(trimmed: &str) -> bool {
    trimmed.starts_with("fn ")
        || trimmed.starts_with("async fn ")
        || trimmed.starts_with("pub fn ")
        || trimmed.starts_with("pub async fn ")
}

pub(super) fn cmd_debt_borrows(file: &Path, json: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let lifetime_debt = lifetime_erasure_debt_report(&source, session.guarantee_policy());

    let tf = kir.transform_facts();
    let mut all_overlaps = Vec::new();
    let total_bindings = tf.bindings.len();
    let mut bindings_with_overlaps = 0usize;

    for (i, binding) in tf.bindings.iter().enumerate() {
        let usage = &tf.usages[i];
        let shared = &tf.shared_facts[i];
        let report = build_borrow_report(&binding.binding_name, usage, shared);
        if !report.overlapping_sites.is_empty() {
            bindings_with_overlaps += 1;
        }
        all_overlaps.extend(report.overlapping_sites);
    }

    let combined = BorrowReport {
        schema_version: 1,
        total_bindings_analyzed: total_bindings,
        bindings_with_overlaps,
        overlapping_sites: all_overlaps,
    };

    if json {
        let json_str = serde_json::to_string_pretty(&combined)
            .context("failed to serialize borrow report to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    // Human-readable output.
    if combined.overlapping_sites.is_empty() {
        println!("No borrow overlaps detected.");
    } else {
        for overlap in &combined.overlapping_sites {
            println!(
                "Borrow overlap in `{}`: {} immutable, {} mutable span(s). Fix: {:?}",
                overlap.binding_name,
                overlap.immutable_spans.len(),
                overlap.mutable_spans.len(),
                overlap.fix_pattern,
            );
        }
        println!(
            "\n{} borrow overlap(s) found.",
            combined.overlapping_sites.len()
        );
    }

    if !lifetime_debt.is_empty() {
        println!("\n{lifetime_debt}");
    }

    Ok(())
}

pub(super) fn cmd_debt_patterns(file: &Path, json: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    let config = GreedyConfig {
        solver_cluster_limit: session.config.solver_cluster_limit,
        solver_budget_seconds: session.config.solver_budget_seconds,
        mutable_sites_threshold: GreedyConfig::default().mutable_sites_threshold,
    };
    let result = greedy_resolve(&kir, &config);
    let patterns = detect_migration_patterns(&kir, &result.resolved);

    if json {
        let json_str = serde_json::to_string_pretty(&patterns)
            .context("failed to serialize patterns to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    let output = format_patterns(&patterns);
    println!("{output}");
    Ok(())
}

/// S-62: Report functions where typed errors would replace boxed/dynamic errors.
///
/// Scans generated code for `Box<dyn Error>`, `anyhow::Error`, and `?` usage
/// in functions that return `Result`. Suggests where typed error enums would help.
pub(super) fn cmd_debt_errors(file: &Path, json: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    // Find functions returning Result and count `?` usage
    let mut entries: Vec<ErrorDebtEntry> = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    let mut current_fn: Option<String> = None;
    let mut question_marks: usize = 0;
    let mut brace_depth: i32 = 0;

    for line in &lines {
        let trimmed = line.trim();

        // Detect function start
        if (trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn "))
            && trimmed.contains("->")
        {
            let fn_name = extract_fn_name_from_line(trimmed);
            if trimmed.contains("Result") {
                current_fn = Some(fn_name);
                question_marks = 0;
                brace_depth = 0;
            }
        }

        if current_fn.is_some() {
            brace_depth += trimmed.matches('{').count() as i32;
            brace_depth -= trimmed.matches('}').count() as i32;
            question_marks += trimmed.matches('?').count();

            if brace_depth <= 0 && trimmed.contains('}') {
                if let Some(fn_name) = current_fn.take() {
                    if question_marks > 0 {
                        let uses_boxed = trimmed.contains("Box<dyn")
                            || source.contains(&format!("fn {fn_name}"))
                                && source.contains("Box<dyn Error");
                        entries.push(ErrorDebtEntry {
                            fn_name,
                            question_mark_count: question_marks,
                            uses_boxed_error: uses_boxed,
                            suggestion: if question_marks > 3 {
                                "consider a typed error enum".to_string()
                            } else {
                                "boxed error is acceptable".to_string()
                            },
                        });
                    }
                }
            }
        }
    }

    if json {
        let values: Vec<serde_json::Value> = entries.iter().map(|e| e.to_json_value()).collect();
        let json_str = serde_json::to_string_pretty(&values)
            .context("failed to serialize error debt to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    if entries.is_empty() {
        println!("No error debt found — no Result-returning functions with `?` usage.");
    } else {
        println!("Error Debt Report");
        println!("═════════════════");
        for entry in &entries {
            println!(
                "  {} — {}x `?` — {}",
                entry.fn_name, entry.question_mark_count, entry.suggestion,
            );
        }
        println!("\n{} function(s) with error debt.", entries.len());
    }

    Ok(())
}

#[derive(Debug)]
struct ErrorDebtEntry {
    fn_name: String,
    question_mark_count: usize,
    uses_boxed_error: bool,
    suggestion: String,
}

impl ErrorDebtEntry {
    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "fn_name": self.fn_name,
            "question_mark_count": self.question_mark_count,
            "uses_boxed_error": self.uses_boxed_error,
            "suggestion": self.suggestion,
        })
    }
}

pub(super) fn extract_fn_name_from_line(line: &str) -> String {
    let rest = line
        .strip_prefix("pub async fn ")
        .or_else(|| line.strip_prefix("async fn "))
        .or_else(|| line.strip_prefix("pub fn "))
        .or_else(|| line.strip_prefix("fn "))
        .unwrap_or(line);
    rest.split('(')
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_string()
}
