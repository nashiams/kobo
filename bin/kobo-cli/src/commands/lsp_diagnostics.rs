use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::run_check_pipeline;
use kobo_errors::{DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{GuaranteePolicy, GuaranteeProfile, KoboSpan};
use serde_json::{json, Value};
use syn::visit::Visit;

use crate::ErrorFormat;

use super::{
    declarations::{self, DeclarationLookup},
    session::build_session,
};
pub(super) fn cmd_lsp_diagnostics(
    file: &Path,
    format: ErrorFormat,
    _no_project_ok: bool,
    include_actions: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(
        file,
        Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
    )?;
    let _ = run_check_pipeline(&mut session, file);
    let mut extra_diagnostics = artifact_backed_diagnostics(&session, file)?;
    extra_diagnostics.extend(replay_boundary_diagnostics(&session, file)?);
    extra_diagnostics.extend(declaration_boundary_diagnostics(&session, file)?);

    match format {
        ErrorFormat::Json => {
            for diagnostic in session.visible_diagnostics() {
                print_lsp_payload(&session, diagnostic, include_actions)?;
            }
            for diagnostic in &extra_diagnostics {
                print_lsp_payload(&session, diagnostic, include_actions)?;
            }
            Ok(())
        }
        ErrorFormat::Human => anyhow::bail!("lsp-diagnostics currently supports --format=json"),
    }
}

fn declaration_boundary_diagnostics(
    session: &kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    let Some((file_id, _)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let source = std::fs::read_to_string(file)?;
    let external_boundary = external_replay_boundary(&source, &session.config);
    let mut diagnostics = Vec::new();
    for mut boundary in source_boundary_policies(&source) {
        let Some(crate_name) = boundary.crate_name.as_deref() else {
            continue;
        };
        if boundary.call_path.is_none() {
            if let Some(external) = external_boundary.as_ref() {
                if external.crate_name == crate_name {
                    boundary.call_path = external.call_path.clone();
                }
            }
        }
        let span = KoboSpan::new(
            boundary.span_start as u32,
            boundary.span_end as u32,
            file_id,
        );
        let expected_version = dependency_version(&session.config, crate_name);
        match declarations::load_declaration(file, crate_name, expected_version.as_deref()) {
            DeclarationLookup::Missing if boundary.policy == "typed" => {
                diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0122,
                    Severity::Error,
                    DiagLabel::primary(span, format!("typed boundary `{crate_name}` has no declaration")),
                    format!(
                        "`{crate_name}` is marked typed, but Kobo could not find a matching kobo.d.toml declaration"
                    ),
                    DiagDecision("add a declaration file or choose record, activity, opaque, or debt".to_owned()),
                ));
            }
            DeclarationLookup::Invalid(error) if boundary.policy == "typed" => {
                diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0121,
                    Severity::Error,
                    DiagLabel::primary(
                        span,
                        format!("invalid declaration metadata for `{crate_name}`"),
                    ),
                    format!(
                        "{} `{}` in {}: {}",
                        "invalid declaration key",
                        error.key,
                        error.path.display(),
                        error.message
                    ),
                    DiagDecision(
                        "fix the declaration file before using typed ecosystem policy".to_owned(),
                    ),
                ));
            }
            DeclarationLookup::Valid(info)
                if boundary.policy == "typed"
                    && !declarations::declaration_covers_call(
                        &info,
                        boundary.call_path.as_deref(),
                    ) =>
            {
                let call_path = boundary
                    .call_path
                    .as_deref()
                    .unwrap_or("<unknown external call>");
                diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0121,
                    Severity::Error,
                    DiagLabel::primary(
                        span,
                        format!("typed declaration for `{crate_name}` does not cover boundary call"),
                    ),
                    format!(
                        "{} does not cover boundary call `{call_path}`",
                        info.path.display()
                    ),
                    DiagDecision(
                        "add function-level declaration evidence for this call or choose another boundary policy".to_owned(),
                    ),
                ));
            }
            DeclarationLookup::Valid(info)
                if boundary.policy == "typed"
                    && declarations::typed_call_has_replay_critical_effects(
                        &info,
                        boundary.call_path.as_deref(),
                    ) =>
            {
                diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0121,
                    Severity::Error,
                    DiagLabel::primary(
                        span,
                        format!("typed declaration for `{crate_name}` still has replay-critical effects"),
                    ),
                    format!(
                        "{} still has replay-critical effects for typed boundary `{crate_name}`",
                        info.path.display()
                    ),
                    DiagDecision(
                        "change this boundary to record/activity/model or mark the function pure/deterministic with no effects".to_owned(),
                    ),
                ));
            }
            DeclarationLookup::Valid(info)
                if boundary.policy == "activity"
                    && !declarations::activity_covers_call(
                        Some(&info),
                        boundary.call_path.as_deref(),
                    ) =>
            {
                diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0125,
                    Severity::Warning,
                    DiagLabel::primary(span, format!("activity boundary `{crate_name}` lacks retry metadata")),
                    format!(
                        "activity declaration for `{crate_name}` in {} does not declare retry and idempotency metadata",
                        info.path.display()
                    ),
                    DiagDecision("record retry, idempotency, and compensation semantics in the declaration".to_owned()),
                ));
            }
            _ => {}
        }
    }
    Ok(diagnostics)
}

fn replay_boundary_diagnostics(
    session: &kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    if session
        .visible_diagnostics()
        .any(|diagnostic| diagnostic.code == KErrorCode::K0107)
    {
        return Ok(Vec::new());
    }
    let source = std::fs::read_to_string(file)?;
    let Some(boundary) = external_replay_boundary(&source, &session.config) else {
        return Ok(Vec::new());
    };
    let Some((file_id, _)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let span = KoboSpan::new(
        boundary.span_start as u32,
        boundary.span_end as u32,
        file_id,
    );
    Ok(vec![KDiagnostic::new(
        KErrorCode::K0107,
        Severity::Warning,
        DiagLabel::primary(
            span,
            format!(
                "unmodeled external boundary `{}`; choose typed, model, record, activity, stub, outside, opaque, or debt",
                boundary.crate_name
            ),
        ),
        format!(
            "unmodeled external boundary `{}`; choose typed, model, record, activity, stub, outside, opaque, or debt. Available policies: typed, model, record, activity, stub, outside, opaque, debt.",
            boundary.crate_name
        ),
        DiagDecision("select an explicit boundary policy for replay-critical evidence".to_owned()),
    )])
}

struct ExternalReplayBoundary {
    crate_name: String,
    call_path: Option<String>,
    span_start: usize,
    span_end: usize,
}

struct SourceBoundaryPolicy {
    crate_name: Option<String>,
    policy: String,
    call_path: Option<String>,
    span_start: usize,
    span_end: usize,
}

fn source_boundary_policies(source: &str) -> Vec<SourceBoundaryPolicy> {
    let Ok(parsed) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut policies = Vec::new();
    for item in &parsed.items {
        for attr in boundary_candidate_attrs(item) {
            if !syn_path_ends_with(attr.path(), &["kobo", "boundary"]) {
                continue;
            }
            if let Some(policy) = parse_source_boundary_policy(attr, source) {
                policies.push(policy);
            }
        }
    }
    policies
}

fn boundary_candidate_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn parse_source_boundary_policy(
    attr: &syn::Attribute,
    source: &str,
) -> Option<SourceBoundaryPolicy> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    let mut crate_name = None;
    let mut policy = None;
    for entry in entries {
        let Some(key) = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
        else {
            continue;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        match key.as_str() {
            "crate" => crate_name = Some(value.value()),
            "policy" => policy = Some(value.value()),
            _ => {}
        }
    }
    let span_start = crate_name
        .as_deref()
        .and_then(|name| source.find(name))
        .or_else(|| source.find("kobo::boundary"))
        .unwrap_or(0);
    let span_end = span_start + crate_name.as_ref().map(String::len).unwrap_or(1);
    Some(SourceBoundaryPolicy {
        crate_name,
        policy: policy.unwrap_or_else(|| "opaque".to_owned()),
        call_path: None,
        span_start,
        span_end,
    })
}

fn dependency_version(config: &kobo_driver::KoboConfig, crate_name: &str) -> Option<String> {
    let value = config.dependencies.get(crate_name)?;
    match value {
        toml::Value::String(version) => Some(version.clone()),
        toml::Value::Table(table) => table
            .get("version")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

fn syn_path_ends_with(path: &syn::Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    segments.len() >= suffix.len()
        && segments[segments.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(left, right)| left == right)
}

fn external_replay_boundary(
    source: &str,
    config: &kobo_driver::KoboConfig,
) -> Option<ExternalReplayBoundary> {
    let parsed = syn::parse_file(source).ok()?;
    let imports = ImportIndex::from_file(&parsed);
    let dependency_names = config
        .dependencies
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut visitor = ExternalBoundaryVisitor {
        source,
        imports,
        dependency_names,
        bindings: std::collections::BTreeMap::new(),
        found: None,
    };
    visitor.visit_file(&parsed);
    visitor.found
}

#[derive(Default)]
struct ImportIndex {
    aliases: std::collections::BTreeMap<String, String>,
    paths: std::collections::BTreeMap<String, Vec<String>>,
}

impl ImportIndex {
    fn from_file(file: &syn::File) -> Self {
        let mut index = Self::default();
        for item in &file.items {
            if let syn::Item::Use(item_use) = item {
                collect_use_tree(&item_use.tree, Vec::new(), &mut index);
            }
        }
        index
    }

    fn path_for_ident(&self, ident: &str) -> Option<&[String]> {
        self.paths.get(ident).map(Vec::as_slice)
    }
}

fn collect_use_tree(tree: &syn::UseTree, prefix: Vec<String>, index: &mut ImportIndex) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            if let Some(root) = next.first() {
                index.aliases.insert(path.ident.to_string(), root.clone());
                index.paths.insert(path.ident.to_string(), next.clone());
            }
            collect_use_tree(&path.tree, next, index);
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix;
            full.push(name.ident.to_string());
            if let Some(root) = full.first() {
                index.aliases.insert(name.ident.to_string(), root.clone());
                index.paths.insert(name.ident.to_string(), full);
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix;
            full.push(rename.ident.to_string());
            if let Some(root) = full.first() {
                index
                    .aliases
                    .insert(rename.rename.to_string(), root.clone());
                index.paths.insert(rename.rename.to_string(), full);
            }
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix.clone(), index);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

struct ExternalBoundaryVisitor<'a> {
    source: &'a str,
    imports: ImportIndex,
    dependency_names: std::collections::BTreeSet<String>,
    bindings: std::collections::BTreeMap<String, Vec<String>>,
    found: Option<ExternalReplayBoundary>,
}

impl<'ast> Visit<'ast> for ExternalBoundaryVisitor<'_> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if self.found.is_some() {
            return;
        }
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let segments = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if let Some((crate_name, call_path)) = self.resolve_call_path(&segments) {
                let needle = call_path.clone().unwrap_or_else(|| segments.join("::"));
                let start = self
                    .source
                    .find(&needle)
                    .or_else(|| self.source.find(&crate_name))
                    .unwrap_or(0);
                self.found = Some(ExternalReplayBoundary {
                    span_start: start,
                    span_end: start + crate_name.len(),
                    crate_name,
                    call_path,
                });
                return;
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some((name, path)) = self.binding_for_local(node) {
            self.bindings.insert(name, path);
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if self.found.is_some() {
            return;
        }
        if let Some((crate_name, call_path)) = self.resolve_method_call_path(node) {
            let start = call_path
                .as_deref()
                .and_then(|path| self.source.find(path))
                .or_else(|| self.source.find(&crate_name))
                .unwrap_or(0);
            self.found = Some(ExternalReplayBoundary {
                span_start: start,
                span_end: start + crate_name.len(),
                crate_name,
                call_path,
            });
            return;
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

impl ExternalBoundaryVisitor<'_> {
    fn resolve_call_path(&self, segments: &[String]) -> Option<(String, Option<String>)> {
        let resolved = self.resolve_segments(segments)?;
        let crate_name = resolved.first()?.clone();
        Some((crate_name, Some(resolved.join("::"))))
    }

    fn resolve_method_call_path(
        &self,
        node: &syn::ExprMethodCall,
    ) -> Option<(String, Option<String>)> {
        let receiver_path = match node.receiver.as_ref() {
            syn::Expr::Path(path) => {
                let segments = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                if segments.len() == 1 {
                    if let Some(binding) = self.bindings.get(&segments[0]) {
                        binding.clone()
                    } else {
                        segments
                    }
                } else {
                    segments
                }
            }
            syn::Expr::Call(call) => {
                let syn::Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                let mut segments = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                segments.pop();
                segments
            }
            _ => return None,
        };
        let mut resolved = self.resolve_segments(&receiver_path)?;
        resolved.push(node.method.to_string());
        let crate_name = resolved.first()?.clone();
        Some((crate_name, Some(resolved.join("::"))))
    }

    fn binding_for_local(&self, node: &syn::Local) -> Option<(String, Vec<String>)> {
        let syn::Pat::Ident(binding) = &node.pat else {
            return None;
        };
        let init = node.init.as_ref()?;
        let syn::Expr::Call(call) = init.expr.as_ref() else {
            return None;
        };
        let syn::Expr::Path(path) = call.func.as_ref() else {
            return None;
        };
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let mut resolved = self.resolve_segments(&segments)?;
        resolved.pop();
        Some((binding.ident.to_string(), resolved))
    }

    fn resolve_segments(&self, segments: &[String]) -> Option<Vec<String>> {
        let first = segments.first()?;
        if self.dependency_names.contains(first) {
            return Some(segments.to_vec());
        }
        if let Some(path) = self.imports.path_for_ident(first) {
            let mut resolved = path.to_vec();
            resolved.extend(segments.iter().skip(1).cloned());
            if let Some(crate_name) = resolved.first() {
                if self.dependency_names.is_empty() || self.dependency_names.contains(crate_name) {
                    return Some(resolved);
                }
            }
        }
        if let Some(crate_name) = self.imports.aliases.get(first) {
            if self.dependency_names.contains(crate_name) {
                let mut resolved = vec![crate_name.clone()];
                resolved.extend(segments.iter().skip(1).cloned());
                return Some(resolved);
            }
        }
        None
    }
}

fn print_lsp_payload(
    session: &kobo_driver::CompileSession,
    diagnostic: &KDiagnostic,
    include_actions: bool,
) -> anyhow::Result<()> {
    let mut value = kobo_lsp::diagnostic_value(session.file_set(), diagnostic, include_actions)?;
    if let Some(line) = session
        .file_set()
        .get(diagnostic.primary.span.file_id)
        .map(|file| file.line_col(diagnostic.primary.span.start).0)
    {
        value["line"] = json!(line);
    }
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

fn artifact_backed_diagnostics(
    session: &kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    let Some((file_id, entry)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let source_hash = kobo_sim_core::digest::stable_hash(entry.source());
    let source_path = cli_relative_path(file)?;
    let witnesses = matching_witnesses(&source_path, &source_hash)?;
    Ok(witnesses
        .into_iter()
        .filter_map(|artifact| diagnostic_from_witness(file_id, entry.source(), artifact).ok())
        .collect())
}

struct WitnessArtifact {
    path: PathBuf,
    json: Value,
}

fn matching_witnesses(
    source_path: &str,
    source_hash: &str,
) -> anyhow::Result<Vec<WitnessArtifact>> {
    let witness_dir = std::env::current_dir()
        .context("failed to determine current directory")?
        .join(".kobo")
        .join("witnesses");
    let Ok(entries) = std::fs::read_dir(&witness_dir) else {
        return Ok(Vec::new());
    };
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("kwit") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if json["source"]["hash"].as_str() != Some(source_hash) {
            continue;
        }
        if json["source"]["path"].as_str() != Some(source_path) {
            continue;
        }
        if json["schema_version"].as_u64() != Some(1) {
            continue;
        }
        if json["replay_guarantee"].as_str() == Some("exact")
            && json["execution_digest"]["agreement"].as_str() != Some("matched")
        {
            continue;
        }
        matches.push(WitnessArtifact { path, json });
    }
    matches.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(matches)
}

fn diagnostic_from_witness(
    file_id: kobo_ir::FileId,
    source: &str,
    artifact: WitnessArtifact,
) -> anyhow::Result<KDiagnostic> {
    let code = parse_code(artifact.json["failure"]["code"].as_str().unwrap_or("K0104"))
        .unwrap_or(KErrorCode::K0104);
    let span = primary_span_from_witness(file_id, source, &artifact.json);
    let message = artifact.json["failure"]["message"]
        .as_str()
        .unwrap_or("simulation evidence failed");
    Ok(KDiagnostic::new(
        code,
        Severity::Error,
        DiagLabel::primary(span, label_for_code(code)),
        message.to_owned(),
        DiagDecision(decision_for_code(code).to_owned()),
    )
    .with_run(format!("kobo replay {}", artifact.path.display())))
}

fn primary_span_from_witness(file_id: kobo_ir::FileId, source: &str, witness: &Value) -> KoboSpan {
    if let Some(start) = witness["obligations"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["declaration_span"]["start"].as_u64())
    {
        let end = witness["obligations"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["declaration_span"]["end"].as_u64())
            .unwrap_or(start + 1);
        return KoboSpan::new(start as u32, end.max(start + 1) as u32, file_id);
    }
    KoboSpan::new(0, source.len().min(1) as u32, file_id)
}

fn label_for_code(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => "must_call liveness token can be dropped",
        KErrorCode::K0102 => "raw nondeterminism appears on replay path",
        KErrorCode::K0105 => "simulation event budget exceeded",
        KErrorCode::K0107 => "external replay boundary requires policy",
        KErrorCode::K0116 => "scenario coverage is incomplete",
        KErrorCode::K0117 => "semantic and harness traces diverged",
        KErrorCode::K0118 => "evidence artifact is stale",
        KErrorCode::K0120 => "ecosystem policy metadata is invalid",
        KErrorCode::K0121 => "declaration metadata is invalid or stale",
        KErrorCode::K0122 => "typed external boundary has no declaration",
        KErrorCode::K0123 => "adapter package is missing or incompatible",
        KErrorCode::K0124 => "record boundary lacks recorded evidence",
        KErrorCode::K0125 => "activity declaration lacks retry metadata",
        KErrorCode::K0126 => ".kobo-summary hash or version mismatched",
        KErrorCode::K0127 => "bindgen declaration draft needs review",
        KErrorCode::K0128 => "Cargo compatibility metadata changed",
        KErrorCode::K0129 => "ecosystem replay evidence would overclaim coverage",
        _ => "simulation contract failed",
    }
}

fn decision_for_code(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => "discharge the must_call action or replay the .kwit witness",
        KErrorCode::K0102 => "route time/random through the deterministic scenario ward",
        KErrorCode::K0105 => "raise the event budget or remove the unbounded scenario loop",
        KErrorCode::K0107 => "select a boundary policy before exact replay",
        KErrorCode::K0116 => "keep the witness partial until the scenario coverage is modeled",
        KErrorCode::K0117 => "regenerate the witness and investigate the trace mismatch",
        KErrorCode::K0118 => "regenerate artifacts for the current source hash",
        KErrorCode::K0120 => "fix the [ecosystem] table before applying boundary policies",
        KErrorCode::K0121 => "fix or regenerate the declaration file",
        KErrorCode::K0122 => "add a declaration file or choose a non-typed boundary policy",
        KErrorCode::K0123 => "install the adapter package or choose record/activity/opaque/debt",
        KErrorCode::K0124 => "regenerate a recorded witness or choose another boundary policy",
        KErrorCode::K0125 => "record retry, idempotency, and compensation metadata",
        KErrorCode::K0126 => "rebuild the upstream package and update the summary hash",
        KErrorCode::K0127 => "review and complete the generated declaration before trusting it",
        KErrorCode::K0128 => "preserve Cargo metadata exactly or surface the Cargo error",
        KErrorCode::K0129 => "keep replay partial or regenerate matching ecosystem evidence",
        _ => "run kobo test --sim quick for the scenario failure",
    }
}

fn parse_code(code: &str) -> Option<KErrorCode> {
    KErrorCode::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == code)
}

fn cli_relative_path(file: &Path) -> anyhow::Result<String> {
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file)
    };
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let display_path = absolute.strip_prefix(&cwd).unwrap_or(&absolute);
    Ok(display_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}
