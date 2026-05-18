use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use anyhow::Context;
use syn::{spanned::Spanned, visit::Visit};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryPolicyProjection {
    pub(super) crate_name: String,
    pub(super) policy: String,
    pub(super) source: &'static str,
    pub(super) calls: Vec<String>,
    pub(super) spans: Vec<BoundarySourceSpan>,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BoundarySourceSpan {
    pub(super) start: usize,
    pub(super) end: usize,
}

impl BoundaryPolicyProjection {
    pub(super) fn inspect_comment(&self) -> String {
        format!(
            "// kobo-boundary-policy: crate={} policy={} source={}{}{}{}",
            self.crate_name,
            self.policy,
            self.source,
            self.call_fragment(),
            self.span_fragment(),
            self.reason
                .as_deref()
                .map(|reason| format!(" reason={reason}"))
                .unwrap_or_default()
        )
    }

    pub(super) fn debt_line(&self) -> String {
        format!(
            "Boundary policy debt: crate={} policy={} source={}{}{}{}",
            self.crate_name,
            self.policy,
            self.source,
            self.call_fragment(),
            self.span_fragment(),
            self.reason
                .as_deref()
                .map(|reason| format!(" reason={reason}"))
                .unwrap_or_default()
        )
    }

    fn call_fragment(&self) -> String {
        if self.calls.is_empty() {
            String::new()
        } else {
            format!(" call={}", self.calls.join(","))
        }
    }

    fn span_fragment(&self) -> String {
        if self.spans.is_empty() {
            String::new()
        } else {
            let spans = self
                .spans
                .iter()
                .map(|span| format!("{}..{}", span.start, span.end))
                .collect::<Vec<_>>()
                .join(",");
            format!(" span={spans}")
        }
    }

    pub(super) fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "crate": self.crate_name,
            "policy": self.policy,
            "source": self.source,
            "calls": self.calls,
            "spans": self.spans.iter().map(|span| {
                serde_json::json!({
                    "start": span.start,
                    "end": span.end,
                })
            }).collect::<Vec<_>>(),
            "reason": self.reason,
        })
    }
}

pub(super) fn projections_for_file(
    file: &Path,
    config: &kobo_driver::KoboConfig,
) -> anyhow::Result<Vec<BoundaryPolicyProjection>> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let references = BoundaryReferences::from_source(&source, config);
    let mut projections = source_boundary_projections(&source, &references);
    let source_crates = projections
        .iter()
        .map(|projection| projection.crate_name.clone())
        .collect::<BTreeSet<_>>();

    for crate_policy in &config.ecosystem_policy.crates {
        projections.push(BoundaryPolicyProjection {
            crate_name: crate_policy.name.clone(),
            policy: crate_policy.policy.as_str().to_owned(),
            source: "project-crate",
            calls: references.calls_for_crate(&crate_policy.name),
            spans: references.spans_for_crate(&crate_policy.name),
            reason: crate_policy.reason.clone(),
        });
    }

    if config.ecosystem_policy.default_is_configured {
        for crate_name in referenced_crates(config, &references) {
            if source_crates.contains(&crate_name)
                || config
                    .ecosystem_policy
                    .crates
                    .iter()
                    .any(|policy| policy.name == crate_name)
            {
                continue;
            }
            projections.push(BoundaryPolicyProjection {
                crate_name: crate_name.clone(),
                policy: config.ecosystem_policy.default.as_str().to_owned(),
                source: "project-default",
                calls: references.calls_for_crate(&crate_name),
                spans: references.spans_for_crate(&crate_name),
                reason: None,
            });
        }
    }

    projections.sort_by(|left, right| {
        (&left.crate_name, left.source, &left.policy).cmp(&(
            &right.crate_name,
            right.source,
            &right.policy,
        ))
    });
    projections.dedup_by(|left, right| {
        left.crate_name == right.crate_name
            && left.source == right.source
            && left.policy == right.policy
            && left.calls == right.calls
            && left.spans == right.spans
    });
    Ok(projections)
}

fn source_boundary_projections(
    source: &str,
    references: &BoundaryReferences,
) -> Vec<BoundaryPolicyProjection> {
    let mut projections = Vec::new();
    let Ok(parsed) = syn::parse_file(source) else {
        return projections;
    };
    for item in &parsed.items {
        for attr in boundary_candidate_attrs(item) {
            if !syn_path_ends_with(attr.path(), &["kobo", "boundary"]) {
                continue;
            }
            if let Some((crate_name, policy, reason)) = parse_boundary_attr(attr) {
                let calls = references.calls_for_crate(&crate_name);
                let spans = references.spans_for_crate(&crate_name);
                projections.push(BoundaryPolicyProjection {
                    crate_name,
                    policy,
                    source: "source",
                    calls,
                    spans,
                    reason,
                });
            }
        }
    }
    projections
}

fn referenced_crates(
    config: &kobo_driver::KoboConfig,
    references: &BoundaryReferences,
) -> BTreeSet<String> {
    let mut crates = references.referenced_crates.clone();
    crates.extend(external_crate_names(config));
    crates
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

fn parse_boundary_attr(attr: &syn::Attribute) -> Option<(String, String, Option<String>)> {
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
    let mut reason = None;
    for entry in entries {
        let key = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
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
            "reason" => reason = Some(value.value()),
            _ => {}
        }
    }
    Some((crate_name?, policy?, reason))
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

#[derive(Debug, Default)]
struct BoundaryReferences {
    referenced_crates: BTreeSet<String>,
    crate_calls: BTreeMap<String, BTreeSet<String>>,
    crate_spans: BTreeMap<String, BTreeSet<BoundarySourceSpan>>,
}

impl BoundaryReferences {
    fn from_source(source: &str, config: &kobo_driver::KoboConfig) -> Self {
        let Ok(file) = syn::parse_file(source) else {
            return Self::default();
        };
        let import_index = ImportIndex::from_file(&file);
        let external_crates = external_crate_names(config);
        let mut references = Self::default();
        for crate_name in import_index.iter_roots() {
            if is_external_root(crate_name) {
                references.referenced_crates.insert(crate_name.to_owned());
            }
        }
        let mut collector = BoundaryReferenceCollector {
            source,
            imports: import_index,
            external_crates,
            bindings: BTreeMap::new(),
            references,
        };
        collector.visit_file(&file);
        collector.references
    }

    fn calls_for_crate(&self, crate_name: &str) -> Vec<String> {
        self.crate_calls
            .get(crate_name)
            .map(|calls| calls.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn spans_for_crate(&self, crate_name: &str) -> Vec<BoundarySourceSpan> {
        self.crate_spans
            .get(crate_name)
            .map(|spans| spans.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, Default)]
struct ImportIndex {
    paths: BTreeMap<String, Vec<String>>,
    roots: BTreeSet<String>,
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

    fn iter_roots(&self) -> impl Iterator<Item = &str> {
        self.roots.iter().map(String::as_str)
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
                index.roots.insert(root.clone());
                index.paths.insert(path.ident.to_string(), next.clone());
            }
            collect_use_tree(&path.tree, next, index);
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix;
            full.push(name.ident.to_string());
            if let Some(root) = full.first() {
                index.roots.insert(root.clone());
                index.paths.insert(name.ident.to_string(), full);
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix;
            full.push(rename.ident.to_string());
            if let Some(root) = full.first() {
                index.roots.insert(root.clone());
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

struct BoundaryReferenceCollector<'src> {
    source: &'src str,
    imports: ImportIndex,
    external_crates: BTreeSet<String>,
    bindings: BTreeMap<String, Vec<String>>,
    references: BoundaryReferences,
}

impl<'ast> Visit<'ast> for BoundaryReferenceCollector<'_> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let segments = path_segments(&path.path);
            if let Some(resolved) = self.resolve_segments(&segments) {
                let span = span_offsets(self.source, path.path.span());
                self.record_call(resolved, span);
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if let Some(mut resolved) = self.receiver_segments(node) {
            resolved.push(node.method.to_string());
            if let Some(resolved) = self.resolve_segments(&resolved) {
                let span = span_offsets(self.source, node.span());
                self.record_call(resolved, span);
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some((binding, resolved_type)) = self.binding_for_local(node) {
            self.bindings.insert(binding, resolved_type);
        }
        syn::visit::visit_local(self, node);
    }
}

impl BoundaryReferenceCollector<'_> {
    fn record_call(&mut self, resolved: Vec<String>, span: Option<BoundarySourceSpan>) {
        let Some(crate_name) = resolved.first().cloned() else {
            return;
        };
        if !is_external_root(&crate_name) {
            return;
        }
        let call_path = resolved.join("::");
        self.references.referenced_crates.insert(crate_name.clone());
        self.references
            .crate_calls
            .entry(crate_name.clone())
            .or_default()
            .insert(call_path.clone());
        if let Some(span) = span {
            self.references
                .crate_spans
                .entry(crate_name)
                .or_default()
                .insert(span);
        }
    }

    fn receiver_segments(&self, node: &syn::ExprMethodCall) -> Option<Vec<String>> {
        match node.receiver.as_ref() {
            syn::Expr::Path(path) => {
                let segments = path_segments(&path.path);
                if segments.len() == 1 {
                    self.bindings.get(&segments[0]).cloned().or(Some(segments))
                } else {
                    Some(segments)
                }
            }
            syn::Expr::Call(call) => {
                let syn::Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                let mut segments = path_segments(&path.path);
                segments.pop();
                Some(segments)
            }
            _ => None,
        }
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
        let mut resolved = self.resolve_segments(&path_segments(&path.path))?;
        resolved.pop();
        Some((binding.ident.to_string(), resolved))
    }

    fn resolve_segments(&self, segments: &[String]) -> Option<Vec<String>> {
        let first = segments.first()?;
        if is_external_root(first) && self.can_accept_root(first) {
            return Some(segments.to_vec());
        }
        if let Some(imported) = self.imports.path_for_ident(first) {
            let mut resolved = imported.to_vec();
            resolved.extend(segments.iter().skip(1).cloned());
            let crate_name = resolved.first()?;
            if is_external_root(crate_name) && self.can_accept_root(crate_name) {
                return Some(resolved);
            }
        }
        None
    }

    fn can_accept_root(&self, crate_name: &str) -> bool {
        self.external_crates.is_empty()
            || self.external_crates.contains(crate_name)
            || self.imports.roots.contains(crate_name)
    }
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn span_offsets(source: &str, span: proc_macro2::Span) -> Option<BoundarySourceSpan> {
    let start = span.start();
    let end = span.end();
    let start_offset = line_column_offset(source, start.line, start.column)?;
    let end_offset = line_column_offset(source, end.line, end.column)?;
    if end_offset <= start_offset {
        return None;
    }
    Some(BoundarySourceSpan {
        start: start_offset,
        end: end_offset,
    })
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

fn external_crate_names(config: &kobo_driver::KoboConfig) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    extend_dependency_names(&mut names, &config.dependencies);
    extend_dependency_names(&mut names, &config.dev_dependencies);
    extend_dependency_names(&mut names, &config.build_dependencies);
    for target in &config.target_dependencies {
        extend_dependency_names(&mut names, &target.dependencies);
        extend_dependency_names(&mut names, &target.dev_dependencies);
        extend_dependency_names(&mut names, &target.build_dependencies);
    }
    names.extend(
        config
            .ecosystem_policy
            .crates
            .iter()
            .map(|policy| policy.name.clone()),
    );
    names.extend(
        config
            .ecosystem_policy
            .types
            .iter()
            .map(|policy| policy.crate_name.clone()),
    );
    names.extend(
        config
            .ecosystem_policy
            .adapters
            .iter()
            .map(|policy| policy.crate_name.clone()),
    );
    names.extend(
        config
            .ecosystem_policy
            .summaries
            .iter()
            .map(|policy| policy.crate_name.clone()),
    );
    names
}

fn extend_dependency_names(
    names: &mut BTreeSet<String>,
    dependencies: &HashMap<String, toml::Value>,
) {
    for (alias, value) in dependencies {
        names.insert(alias.clone());
        if let toml::Value::Table(table) = value {
            if let Some(package) = table.get("package").and_then(toml::Value::as_str) {
                names.insert(package.to_owned());
            }
        }
    }
}

fn is_external_root(crate_name: &str) -> bool {
    !crate_name.is_empty()
        && crate_name != "crate"
        && crate_name != "self"
        && crate_name != "super"
        && crate_name != "std"
}
