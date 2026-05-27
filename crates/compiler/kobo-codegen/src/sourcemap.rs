// Source map precision: LINE_LEVEL
// Column fields in RsSpan are line-boundary approximations (0..line_len).
// Reason: prettyplease::unparse returns a String with no token positions.
// Token-level precision requires post-format scanning; currently source maps stay span-based.
// Upgrade path: replace line-level binding lookup with token scanning when it lands.

use std::{collections::BTreeMap, path::Path};

use kobo_ir::{
    KoboSpan, OwnershipTier, ScenarioCoreTerminatorKind, ScenarioLifecycleTemplate,
    ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::lower::{LoweringSite, ResolvedAnchorMap};
use crate::RuntimeEvidence;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RsSpan {
    pub line: usize,
    pub column_start: usize,
    pub column_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMapEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_event_id: Option<String>,
    pub binding_name: String,
    pub rs_span: RsSpan,
    pub kobo_span: KoboSpan,
    pub ownership_tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_node_id: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoweringTraceEvent {
    pub id: String,
    pub core_event_id: String,
    pub function: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    pub order: u64,
    pub source_map_entry_id: String,
    pub rs_span: RsSpan,
    pub kobo_span: KoboSpan,
    pub lowering_phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolverEvidenceJson {
    pub outcome: String,
    pub graph_fingerprint: String,
    pub node_count: u64,
    pub edge_count: u64,
    pub budget: SolverBudgetJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolverBudgetJson {
    pub max_cluster_size: u64,
    pub budget_seconds: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KoboSourceMap {
    pub version: u32,
    pub file: String,
    pub sources: Vec<String>,
    #[serde(rename = "x_kobo_mappings")]
    pub x_kobo_mappings: Vec<SourceMapEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_evidence: Option<RuntimeEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_evidence: Option<SolverEvidenceJson>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lowering_trace: Vec<LoweringTraceEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedProofEventRole {
    Create,
    Discharge,
    Transfer,
    Move,
    Escape,
    ReturnFunction,
    ReturnTerminator,
    ErrorExit,
    Panic,
    Cancel,
    OpaqueBoundary,
}

#[derive(Clone, Debug)]
struct GeneratedProofEventAnchor {
    function: String,
    kind: &'static str,
    binding: Option<String>,
    detail: Option<String>,
    role: GeneratedProofEventRole,
    rs_span: RsSpan,
}

#[derive(Default)]
struct GeneratedEventCollector {
    function: String,
    anchors: Vec<GeneratedProofEventAnchor>,
}

pub(crate) fn build_source_map_entries(
    sites: &[LoweringSite],
    anchors: &ResolvedAnchorMap,
) -> Vec<SourceMapEntry> {
    let mut entries = Vec::with_capacity(sites.len());

    for site in sites {
        let anchor = anchors.get(site.node).unwrap_or_else(|| {
            panic!(
                "invariant: every lowering site must resolve to an anchor; missing node {:?} binding `{}` span {:?}",
                site.node, site.binding_name, site.kobo_span
            )
        });

        entries.push(SourceMapEntry {
            id: format!("map-{}", site.node.0),
            core_event_id: None,
            binding_name: site.binding_name.clone(),
            rs_span: RsSpan {
                line: anchor.line,
                column_start: anchor.column_start,
                column_end: anchor.column_end,
            },
            kobo_span: site.kobo_span,
            ownership_tier: ownership_tier_label(site.ownership_tier).to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: Some(site.node.0 as u64),
        });
    }

    entries
}

pub(crate) fn add_proof_event_source_entries(
    entries: &mut Vec<SourceMapEntry>,
    rs_source: &str,
    programs: &[ScenarioProgram],
) {
    let generated_anchors = generated_proof_event_anchors(rs_source);
    let mut used_anchors = vec![false; generated_anchors.len()];
    for program in programs {
        let template_by_binding = template_by_binding(program);
        for (order, operation) in program.operations.iter().enumerate() {
            let Some(event) = lowering_event_from_operation(&operation.kind, &template_by_binding)
            else {
                continue;
            };
            let core_event_id =
                core_event_id_for_operation(&program.target, order, &operation.kind);
            let source_map_entry_id = proof_source_map_entry_id(&program.target, order);
            if has_core_event_anchor(entries, &source_map_entry_id, &core_event_id) {
                continue;
            }
            let Some(rs_span) = generated_anchor_for_operation(
                &generated_anchors,
                &mut used_anchors,
                program,
                &operation.kind,
                &event,
            ) else {
                continue;
            };
            entries.push(SourceMapEntry {
                id: source_map_entry_id,
                core_event_id: Some(core_event_id),
                binding_name: event
                    .binding
                    .clone()
                    .unwrap_or_else(|| event.kind.to_owned()),
                rs_span,
                kobo_span: operation.span,
                ownership_tier: "proof-event".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            });
        }
    }
}

pub fn wrap_source_map(
    kobo_path: &Path,
    rs_path: &Path,
    entries: Vec<SourceMapEntry>,
) -> KoboSourceMap {
    KoboSourceMap {
        version: 3,
        file: rs_path.display().to_string(),
        sources: vec![kobo_path.display().to_string()],
        x_kobo_mappings: entries,
        runtime_evidence: None,
        solver_evidence: None,
        lowering_trace: Vec::new(),
    }
}

fn has_core_event_anchor(
    entries: &[SourceMapEntry],
    source_map_entry_id: &str,
    core_event_id: &str,
) -> bool {
    entries.iter().any(|entry| {
        entry.id == source_map_entry_id && entry.core_event_id.as_deref() == Some(core_event_id)
    })
}

fn generated_anchor_for_operation(
    anchors: &[GeneratedProofEventAnchor],
    used_anchors: &mut [bool],
    program: &ScenarioProgram,
    operation: &ScenarioOpKind,
    event: &TraceEventSource<'_>,
) -> Option<RsSpan> {
    let role = generated_anchor_role(operation)?;
    let binding = event.binding.as_deref();
    let detail = generated_anchor_detail(operation);
    let (index, anchor) = anchors.iter().enumerate().find(|(index, anchor)| {
        !used_anchors[*index]
            && anchor.function == program.target
            && anchor.kind == event.kind
            && anchor.binding.as_deref() == binding
            && anchor.detail == detail
            && anchor.role == role
    })?;
    used_anchors[index] = true;
    Some(anchor.rs_span.clone())
}

fn generated_anchor_detail(operation: &ScenarioOpKind) -> Option<String> {
    match operation {
        ScenarioOpKind::Discharge { action, .. } => Some(discharge_anchor_detail(action)),
        ScenarioOpKind::Transfer { callee, .. } => Some(callee.clone()),
        ScenarioOpKind::ExternalBoundary {
            call_path,
            crate_name,
            ..
        } => Some(
            call_path
                .as_deref()
                .and_then(last_path_segment_text)
                .unwrap_or(crate_name)
                .to_owned(),
        ),
        ScenarioOpKind::CoreTerminator {
            kind: ScenarioCoreTerminatorKind::OpaqueBoundary,
            boundary,
            ..
        } => boundary.clone(),
        _ => None,
    }
}

fn discharge_anchor_detail(action: &str) -> String {
    if action.starts_with("escape:") {
        return "escape".to_owned();
    }
    if action.starts_with("suppressed:") {
        return "suppressed".to_owned();
    }
    action.to_owned()
}

fn last_path_segment_text(path: &str) -> Option<&str> {
    path.rsplit("::")
        .next()
        .filter(|segment| !segment.is_empty())
}

fn generated_anchor_role(operation: &ScenarioOpKind) -> Option<GeneratedProofEventRole> {
    match operation {
        ScenarioOpKind::CreateObligation { .. } => Some(GeneratedProofEventRole::Create),
        ScenarioOpKind::Discharge { .. } => Some(GeneratedProofEventRole::Discharge),
        ScenarioOpKind::Transfer { .. } => Some(GeneratedProofEventRole::Transfer),
        ScenarioOpKind::MoveBinding { .. } => Some(GeneratedProofEventRole::Move),
        ScenarioOpKind::ExternalBoundary { .. } => Some(GeneratedProofEventRole::Escape),
        ScenarioOpKind::Return => Some(GeneratedProofEventRole::ReturnFunction),
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(core_terminator_anchor_role(kind)),
        _ => None,
    }
}

fn core_terminator_anchor_role(kind: &ScenarioCoreTerminatorKind) -> GeneratedProofEventRole {
    match kind {
        ScenarioCoreTerminatorKind::Return => GeneratedProofEventRole::ReturnTerminator,
        ScenarioCoreTerminatorKind::ErrorExit => GeneratedProofEventRole::ErrorExit,
        ScenarioCoreTerminatorKind::Panic => GeneratedProofEventRole::Panic,
        ScenarioCoreTerminatorKind::Await => GeneratedProofEventRole::Cancel,
        ScenarioCoreTerminatorKind::OpaqueBoundary => GeneratedProofEventRole::OpaqueBoundary,
    }
}

fn generated_proof_event_anchors(rs_source: &str) -> Vec<GeneratedProofEventAnchor> {
    let Ok(file) = syn::parse_file(rs_source) else {
        return Vec::new();
    };
    let mut anchors = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(function) => {
                anchors.extend(collect_function_event_anchors(
                    function.sig.ident.to_string(),
                    &function.sig.inputs,
                    &function.block,
                    function.sig.ident.span(),
                ));
            }
            syn::Item::Impl(item_impl) => {
                for impl_item in &item_impl.items {
                    let syn::ImplItem::Fn(function) = impl_item else {
                        continue;
                    };
                    anchors.extend(collect_function_event_anchors(
                        function.sig.ident.to_string(),
                        &function.sig.inputs,
                        &function.block,
                        function.sig.ident.span(),
                    ));
                }
            }
            _ => {}
        }
    }
    anchors
}

fn collect_function_event_anchors(
    function: String,
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
    block: &syn::Block,
    function_span: proc_macro2::Span,
) -> Vec<GeneratedProofEventAnchor> {
    let mut collector = GeneratedEventCollector {
        function,
        anchors: Vec::new(),
    };
    collector.push_anchor(
        "return",
        None,
        None,
        GeneratedProofEventRole::ReturnFunction,
        function_span,
    );
    for input in inputs {
        let syn::FnArg::Typed(argument) = input else {
            continue;
        };
        let Some(binding) = binding_ident(&argument.pat) else {
            continue;
        };
        collector.push_anchor(
            "create",
            Some(binding.to_string()),
            None,
            GeneratedProofEventRole::Create,
            binding.span(),
        );
    }
    collector.visit_block(block);
    collector.anchors
}

impl GeneratedEventCollector {
    fn push_anchor(
        &mut self,
        kind: &'static str,
        binding: Option<String>,
        detail: Option<String>,
        role: GeneratedProofEventRole,
        span: proc_macro2::Span,
    ) {
        self.anchors.push(GeneratedProofEventAnchor {
            function: self.function.clone(),
            kind,
            binding,
            detail,
            role,
            rs_span: rs_span_from_syn(span),
        });
    }
}

impl<'ast> Visit<'ast> for GeneratedEventCollector {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(init) = &local.init {
            if let Some(binding) = binding_ident(&local.pat) {
                self.push_anchor(
                    "create",
                    Some(binding.to_string()),
                    None,
                    GeneratedProofEventRole::Create,
                    event_expr_span(init.expr.as_ref()),
                );
            }
            if let Some(binding) = expr_path_ident(init.expr.as_ref()) {
                self.push_anchor(
                    "move",
                    Some(binding),
                    None,
                    GeneratedProofEventRole::Move,
                    event_expr_span(init.expr.as_ref()),
                );
            }
        }
        visit::visit_local(self, local);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if let Some(binding) = receiver_ident(call.receiver.as_ref()) {
            self.push_anchor(
                "discharge",
                Some(binding),
                Some(terminal_action_name(&call.method.to_string())),
                GeneratedProofEventRole::Discharge,
                call.span(),
            );
        }
        let method_detail = Some(call.method.to_string());
        self.push_anchor(
            "escape",
            None,
            method_detail.clone(),
            GeneratedProofEventRole::Escape,
            call.span(),
        );
        self.push_anchor(
            "opaque_boundary",
            None,
            method_detail,
            GeneratedProofEventRole::OpaqueBoundary,
            call.span(),
        );
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let call_detail = call_detail(call);
        for argument in &call.args {
            if let Some(binding) = expr_path_ident(argument) {
                self.push_anchor(
                    "discharge",
                    Some(binding.clone()),
                    Some("escape".to_owned()),
                    GeneratedProofEventRole::Discharge,
                    call.span(),
                );
                self.push_anchor(
                    "transfer",
                    Some(binding),
                    call_detail.clone(),
                    GeneratedProofEventRole::Transfer,
                    call.span(),
                );
            }
        }
        self.push_anchor(
            "escape",
            None,
            call_detail.clone(),
            GeneratedProofEventRole::Escape,
            call.span(),
        );
        self.push_anchor(
            "opaque_boundary",
            None,
            call_detail,
            GeneratedProofEventRole::OpaqueBoundary,
            call.span(),
        );
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_try(&mut self, expr_try: &'ast syn::ExprTry) {
        self.push_anchor(
            "error_exit",
            None,
            None,
            GeneratedProofEventRole::ErrorExit,
            event_expr_span(expr_try.expr.as_ref()),
        );
        visit::visit_expr_try(self, expr_try);
    }

    fn visit_expr_return(&mut self, expr_return: &'ast syn::ExprReturn) {
        self.push_anchor(
            "return",
            None,
            None,
            GeneratedProofEventRole::ReturnTerminator,
            expr_return.span(),
        );
        visit::visit_expr_return(self, expr_return);
    }

    fn visit_expr_await(&mut self, expr_await: &'ast syn::ExprAwait) {
        self.push_anchor(
            "cancel",
            None,
            None,
            GeneratedProofEventRole::Cancel,
            expr_await.span(),
        );
        visit::visit_expr_await(self, expr_await);
    }

    fn visit_expr_macro(&mut self, expr_macro: &'ast syn::ExprMacro) {
        self.record_macro(&expr_macro.mac);
        visit::visit_expr_macro(self, expr_macro);
    }

    fn visit_stmt_macro(&mut self, stmt_macro: &'ast syn::StmtMacro) {
        self.record_macro(&stmt_macro.mac);
        visit::visit_stmt_macro(self, stmt_macro);
    }
}

impl GeneratedEventCollector {
    fn record_macro(&mut self, mac: &syn::Macro) {
        if path_last_ident(&mac.path).as_deref() == Some("panic") {
            self.push_anchor(
                "panic",
                None,
                None,
                GeneratedProofEventRole::Panic,
                mac.span(),
            );
        }
    }
}

fn terminal_action_name(method: &str) -> String {
    match method {
        "detach_with_policy" => "detach-with-policy".to_owned(),
        other => other.to_owned(),
    }
}

fn call_detail(call: &syn::ExprCall) -> Option<String> {
    match call.func.as_ref() {
        syn::Expr::Path(path) => path_last_ident(&path.path),
        _ => None,
    }
}

fn event_expr_span(expr: &syn::Expr) -> proc_macro2::Span {
    first_call_like_span(expr).unwrap_or_else(|| expr.span())
}

fn first_call_like_span(expr: &syn::Expr) -> Option<proc_macro2::Span> {
    match expr {
        syn::Expr::Call(call) => Some(call.span()),
        syn::Expr::MethodCall(call) => Some(call.span()),
        syn::Expr::Block(block) => block
            .block
            .stmts
            .iter()
            .find_map(first_call_like_span_from_stmt),
        syn::Expr::Paren(paren) => first_call_like_span(paren.expr.as_ref()),
        syn::Expr::Try(expr_try) => first_call_like_span(expr_try.expr.as_ref()),
        syn::Expr::Await(await_expr) => first_call_like_span(await_expr.base.as_ref()),
        _ => None,
    }
}

fn first_call_like_span_from_stmt(stmt: &syn::Stmt) -> Option<proc_macro2::Span> {
    match stmt {
        syn::Stmt::Expr(expr, _) => first_call_like_span(expr),
        syn::Stmt::Local(local) => local
            .init
            .as_ref()
            .and_then(|init| first_call_like_span(init.expr.as_ref())),
        _ => None,
    }
}

fn rs_span_from_syn(span: proc_macro2::Span) -> RsSpan {
    let start = span.start();
    let end = span.end();
    RsSpan {
        line: start.line,
        column_start: start.column,
        column_end: end.column.max(start.column + 1),
    }
}

fn binding_ident(pattern: &syn::Pat) -> Option<&syn::Ident> {
    match pattern {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        syn::Pat::Type(typed) => binding_ident(&typed.pat),
        _ => None,
    }
}

fn expr_path_ident(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) => path_last_ident(&path.path),
        syn::Expr::Paren(paren) => expr_path_ident(paren.expr.as_ref()),
        _ => None,
    }
}

fn receiver_ident(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) => path_last_ident(&path.path),
        syn::Expr::Paren(paren) => receiver_ident(paren.expr.as_ref()),
        _ => None,
    }
}

fn path_last_ident(path: &syn::Path) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(crate) fn build_lowering_trace(
    programs: &[ScenarioProgram],
    source_map: &KoboSourceMap,
) -> Vec<LoweringTraceEvent> {
    programs
        .iter()
        .flat_map(|program| {
            let template_by_binding = template_by_binding(program);
            program
                .operations
                .iter()
                .enumerate()
                .filter_map(move |(order, operation)| {
                    let event =
                        lowering_event_from_operation(&operation.kind, &template_by_binding)?;
                    let core_event_id =
                        core_event_id_for_operation(&program.target, order, &operation.kind);
                    let source_map_entry_id = proof_source_map_entry_id(&program.target, order);
                    let anchor =
                        anchor_for_trace_event(source_map, &source_map_entry_id, &core_event_id)?;
                    Some(LoweringTraceEvent {
                        id: format!("lowering-{}-{order}", program.target),
                        core_event_id,
                        function: program.target.clone(),
                        kind: event.kind.to_owned(),
                        binding: event.binding,
                        order: order as u64,
                        source_map_entry_id: anchor.id.clone(),
                        rs_span: anchor.rs_span.clone(),
                        kobo_span: anchor.kobo_span,
                        lowering_phase: "kobo-codegen".to_owned(),
                        template_id: event.template.map(|template| template.id.clone()),
                        template_version: event.template.map(template_version),
                    })
                })
        })
        .collect()
}

fn proof_source_map_entry_id(function: &str, order: usize) -> String {
    format!("proof-map-{function}-{order}")
}

fn core_event_id_for_operation(function: &str, order: usize, kind: &ScenarioOpKind) -> String {
    if matches!(
        kind,
        ScenarioOpKind::CreateObligation { .. }
            | ScenarioOpKind::Discharge { .. }
            | ScenarioOpKind::Transfer { .. }
            | ScenarioOpKind::MoveBinding { .. }
            | ScenarioOpKind::ExternalBoundary { .. }
    ) {
        return format!("core-{function}-stmt-{order}");
    }
    format!("core-{function}-term-{order}")
}

struct TraceEventSource<'a> {
    kind: &'static str,
    binding: Option<String>,
    template: Option<&'a ScenarioLifecycleTemplate>,
}

fn lowering_event_from_operation<'a>(
    kind: &'a ScenarioOpKind,
    template_by_binding: &BTreeMap<&'a str, &'a ScenarioLifecycleTemplate>,
) -> Option<TraceEventSource<'a>> {
    match kind {
        ScenarioOpKind::CreateObligation {
            binding, template, ..
        } => Some(TraceEventSource {
            kind: "create",
            binding: Some(binding.clone()),
            template: template
                .as_ref()
                .or_else(|| template_by_binding.get(binding.as_str()).copied()),
        }),
        ScenarioOpKind::Discharge { binding, .. } => Some(binding_event_source(
            "discharge",
            binding,
            template_by_binding,
        )),
        ScenarioOpKind::Transfer { binding, .. } => Some(binding_event_source(
            "transfer",
            binding,
            template_by_binding,
        )),
        ScenarioOpKind::MoveBinding { binding } => {
            Some(binding_event_source("move", binding, template_by_binding))
        }
        ScenarioOpKind::ExternalBoundary { .. } => Some(TraceEventSource {
            kind: "escape",
            binding: None,
            template: None,
        }),
        ScenarioOpKind::Return => Some(TraceEventSource {
            kind: "return",
            binding: None,
            template: None,
        }),
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(TraceEventSource {
            kind: core_terminator_trace_kind(kind),
            binding: None,
            template: None,
        }),
        _ => None,
    }
}

fn binding_event_source<'a>(
    kind: &'static str,
    binding: &'a str,
    template_by_binding: &BTreeMap<&'a str, &'a ScenarioLifecycleTemplate>,
) -> TraceEventSource<'a> {
    TraceEventSource {
        kind,
        binding: Some(binding.to_owned()),
        template: template_by_binding.get(binding).copied(),
    }
}

fn template_by_binding(program: &ScenarioProgram) -> BTreeMap<&str, &ScenarioLifecycleTemplate> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding,
                template: Some(template),
                ..
            } => Some((binding.as_str(), template)),
            _ => None,
        })
        .collect()
}

fn core_terminator_trace_kind(kind: &ScenarioCoreTerminatorKind) -> &'static str {
    match kind {
        ScenarioCoreTerminatorKind::Return => "return",
        ScenarioCoreTerminatorKind::ErrorExit => "error_exit",
        ScenarioCoreTerminatorKind::Panic => "panic",
        ScenarioCoreTerminatorKind::Await => "cancel",
        ScenarioCoreTerminatorKind::OpaqueBoundary => "opaque_boundary",
    }
}

fn anchor_for_trace_event<'a>(
    source_map: &'a KoboSourceMap,
    source_map_entry_id: &str,
    core_event_id: &str,
) -> Option<&'a SourceMapEntry> {
    source_map.x_kobo_mappings.iter().find(|entry| {
        entry.id == source_map_entry_id && entry.core_event_id.as_deref() == Some(core_event_id)
    })
}

fn template_version(template: &ScenarioLifecycleTemplate) -> String {
    if matches!(template.source, ScenarioLifecycleTemplateSource::Inference) {
        "0.1".to_owned()
    } else {
        template.schema_version.to_string()
    }
}

impl KoboSourceMap {
    pub fn generated_file(&self) -> &str {
        &self.file
    }

    pub fn kobo_path(&self) -> &str {
        self.sources
            .first()
            .map(String::as_str)
            .unwrap_or(self.file.as_str())
    }

    pub fn lookup_kobo_span(&self, rs_span: RsSpan) -> Option<KoboSpan> {
        self.x_kobo_mappings
            .iter()
            .find(|entry| {
                entry.rs_span.line == rs_span.line && spans_overlap(&entry.rs_span, &rs_span)
            })
            .map(|entry| entry.kobo_span)
            .or_else(|| {
                self.x_kobo_mappings
                    .iter()
                    .find(|entry| entry.rs_span.line == rs_span.line)
                    .map(|entry| entry.kobo_span)
            })
    }

    pub fn lookup_rs_spans(&self, kobo_span: KoboSpan) -> Vec<RsSpan> {
        self.x_kobo_mappings
            .iter()
            .filter(|entry| entry.kobo_span.overlaps(kobo_span) || entry.kobo_span == kobo_span)
            .map(|entry| entry.rs_span.clone())
            .collect()
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

fn spans_overlap(left: &RsSpan, right: &RsSpan) -> bool {
    left.column_start <= right.column_end && right.column_start <= left.column_end
}

#[cfg(test)]
mod lowering_trace_tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use kobo_ir::{
        FileId, ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioCoreTerminatorKind,
        ScenarioCoverageFacts, ScenarioExternalCallShape, ScenarioLifecycleTemplate, ScenarioOp,
        ScenarioOpKind, ScenarioProgram,
    };

    use super::{build_lowering_trace, wrap_source_map, RsSpan, SourceMapEntry};

    #[test]
    fn lowering_trace_schema_covers_translation_event_kinds() {
        let file_id = FileId(1);
        let span = kobo_ir::KoboSpan::new(10, 20, file_id);
        let operations = trace_kind_operations(span);
        let program = ScenarioProgram {
            file_id,
            target: "trace_case".to_owned(),
            source_hash: "source".to_owned(),
            operations,
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };
        let entries = program
            .operations
            .iter()
            .enumerate()
            .map(|(order, operation)| SourceMapEntry {
                id: format!("proof-map-trace_case-{order}"),
                core_event_id: Some(super::core_event_id_for_operation(
                    "trace_case",
                    order,
                    &operation.kind,
                )),
                binding_name: "delivery".to_owned(),
                kobo_span: span,
                rs_span: RsSpan {
                    line: order + 1,
                    column_start: 1,
                    column_end: 8,
                },
                ownership_tier: "proof-event".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            })
            .collect();
        let source_map = wrap_source_map(
            Path::new("src/main.kobo"),
            Path::new("src/main.rs"),
            entries,
        );
        let kinds = build_lowering_trace(&[program], &source_map)
            .into_iter()
            .map(|event| event.kind)
            .collect::<BTreeSet<_>>();

        for kind in [
            "create",
            "move",
            "transfer",
            "discharge",
            "return",
            "escape",
            "panic",
            "error_exit",
            "cancel",
            "opaque_boundary",
        ] {
            assert!(
                kinds.contains(kind),
                "missing lowering trace kind {kind}: {kinds:?}"
            );
        }
    }

    #[test]
    fn lowering_trace_does_not_invent_anchor_from_same_binding() {
        let file_id = FileId(1);
        let mapped_span = kobo_ir::KoboSpan::new(10, 20, file_id);
        let unmapped_span = kobo_ir::KoboSpan::new(40, 50, file_id);
        let source_map = wrap_source_map(
            Path::new("src/main.kobo"),
            Path::new("src/main.rs"),
            vec![SourceMapEntry {
                id: "map-0".to_owned(),
                core_event_id: None,
                binding_name: "delivery".to_owned(),
                kobo_span: mapped_span,
                rs_span: RsSpan {
                    line: 1,
                    column_start: 1,
                    column_end: 8,
                },
                ownership_tier: "plain".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            }],
        );
        let program = ScenarioProgram {
            file_id,
            target: "trace_case".to_owned(),
            source_hash: "source".to_owned(),
            operations: vec![ScenarioOp {
                span: unmapped_span,
                kind: ScenarioOpKind::Discharge {
                    binding: "delivery".to_owned(),
                    action: "ack".to_owned(),
                },
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };

        let trace = build_lowering_trace(&[program], &source_map);

        assert!(
            trace.is_empty(),
            "unmapped proof events must not reuse a same-binding anchor: {trace:?}"
        );
    }

    #[test]
    fn map_entry_without_core_event_id_does_not_anchor_generated_trace() {
        let file_id = FileId(1);
        let span = kobo_ir::KoboSpan::new(10, 20, file_id);
        let source_map = wrap_source_map(
            Path::new("src/main.kobo"),
            Path::new("src/main.rs"),
            vec![SourceMapEntry {
                id: "map-0".to_owned(),
                core_event_id: None,
                binding_name: "delivery".to_owned(),
                kobo_span: span,
                rs_span: RsSpan {
                    line: 1,
                    column_start: 1,
                    column_end: 8,
                },
                ownership_tier: "plain".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            }],
        );
        let program = ScenarioProgram {
            file_id,
            target: "trace_case".to_owned(),
            source_hash: "source".to_owned(),
            operations: vec![ScenarioOp {
                span,
                kind: ScenarioOpKind::Discharge {
                    binding: "delivery".to_owned(),
                    action: "ack".to_owned(),
                },
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };

        let trace = build_lowering_trace(&[program], &source_map);

        assert!(
            trace.is_empty(),
            "generated proof trace must require an anchor with matching core_event_id: {trace:?}"
        );
    }

    #[test]
    fn lowering_trace_core_event_ids_are_function_scoped() {
        let file_id = FileId(1);
        let first_span = kobo_ir::KoboSpan::new(10, 20, file_id);
        let second_span = kobo_ir::KoboSpan::new(40, 50, file_id);
        let source_map = wrap_source_map(
            Path::new("src/main.kobo"),
            Path::new("src/main.rs"),
            vec![
                SourceMapEntry {
                    id: "proof-map-first-0".to_owned(),
                    core_event_id: Some("core-first-stmt-0".to_owned()),
                    binding_name: "delivery".to_owned(),
                    kobo_span: first_span,
                    rs_span: RsSpan {
                        line: 1,
                        column_start: 1,
                        column_end: 8,
                    },
                    ownership_tier: "proof-event".to_owned(),
                    solver_outcome: None,
                    decision_source: None,
                    solver_node_id: None,
                },
                SourceMapEntry {
                    id: "proof-map-second-0".to_owned(),
                    core_event_id: Some("core-second-stmt-0".to_owned()),
                    binding_name: "delivery".to_owned(),
                    kobo_span: second_span,
                    rs_span: RsSpan {
                        line: 2,
                        column_start: 1,
                        column_end: 8,
                    },
                    ownership_tier: "proof-event".to_owned(),
                    solver_outcome: None,
                    decision_source: None,
                    solver_node_id: None,
                },
            ],
        );
        let first_program = single_discharge_program(file_id, "first", first_span);
        let second_program = single_discharge_program(file_id, "second", second_span);

        let trace = build_lowering_trace(&[first_program, second_program], &source_map);
        let core_event_ids = trace
            .iter()
            .map(|event| event.core_event_id.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            trace.len(),
            2,
            "both functions need distinct anchors: {trace:?}"
        );
        assert!(
            core_event_ids.contains("core-first-stmt-0"),
            "first function event id must be function-scoped: {trace:?}"
        );
        assert!(
            core_event_ids.contains("core-second-stmt-0"),
            "second function event id must be function-scoped: {trace:?}"
        );
    }

    #[test]
    fn source_map_without_event_site_anchor_omits_generated_trace_event() {
        let file_id = FileId(1);
        let mapped_span = kobo_ir::KoboSpan::new(10, 20, file_id);
        let event_span = kobo_ir::KoboSpan::new(40, 50, file_id);
        let entries = vec![SourceMapEntry {
            id: "map-0".to_owned(),
            core_event_id: None,
            binding_name: "delivery".to_owned(),
            kobo_span: mapped_span,
            rs_span: RsSpan {
                line: 1,
                column_start: 1,
                column_end: 8,
            },
            ownership_tier: "plain".to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: None,
        }];
        let program = ScenarioProgram {
            file_id,
            target: "trace_case".to_owned(),
            source_hash: "source".to_owned(),
            operations: vec![ScenarioOp {
                span: event_span,
                kind: ScenarioOpKind::Discharge {
                    binding: "delivery".to_owned(),
                    action: "ack".to_owned(),
                },
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };

        assert!(
            entries
                .iter()
                .all(|entry| !entry.id.starts_with("trace-map-")),
            "unmapped proof events must not synthesize copied trace-map anchors: {entries:?}"
        );
        let source_map = wrap_source_map(
            Path::new("src/main.kobo"),
            Path::new("src/main.rs"),
            entries,
        );
        let trace = build_lowering_trace(&[program], &source_map);

        assert!(
            trace.is_empty(),
            "unmapped proof events must not produce generated trace events: {trace:?}"
        );
    }

    #[test]
    fn proof_event_anchors_use_generated_function_scope_not_global_snippet() {
        let source = r#"
fn fallible() -> Result<(), ()> { Ok(()) }

fn first() -> Result<(), ()> {
    fallible()?;
    Ok(())
}

fn second() -> Result<(), ()> {
    fallible()?;
    Ok(())
}
"#;
        let operation_span = span_for(source, "fn second", "fallible()?");
        let rs_source = r#"fn fallible() -> Result<(), ()> {
    Ok(())
}
fn first() -> Result<(), ()> {
    {
        fallible()
    }?;
    Ok(())
}
fn second() -> Result<(), ()> {
    {
        fallible()
    }?;
    Ok(())
}
"#;
        let mut entries = Vec::new();
        let program = ScenarioProgram {
            file_id: FileId(1),
            target: "second".to_owned(),
            source_hash: "source".to_owned(),
            operations: vec![ScenarioOp {
                span: operation_span,
                kind: ScenarioOpKind::CoreTerminator {
                    kind: ScenarioCoreTerminatorKind::ErrorExit,
                    boundary: None,
                    policy: None,
                    edges: vec!["error_exit".to_owned()],
                },
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };

        super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

        let second_function_line = line_containing(rs_source, "fn second");
        let entry = entries
            .iter()
            .find(|entry| entry.id == "proof-map-second-0")
            .expect("proof event should get a generated anchor");
        assert!(
            entry.rs_span.line > second_function_line,
            "proof anchor must be in second function, not a global text match: {entry:?}"
        );
    }

    #[test]
    fn discharge_anchors_match_action_not_any_same_binding_method() {
        let rs_source = r#"fn case() {
    delivery.inspect();
    delivery.ack();
}
"#;
        let mut entries = Vec::new();
        let program = ScenarioProgram {
            file_id: FileId(1),
            target: "case".to_owned(),
            source_hash: "source".to_owned(),
            operations: vec![ScenarioOp {
                span: kobo_ir::KoboSpan::new(20, 34, FileId(1)),
                kind: ScenarioOpKind::Discharge {
                    binding: "delivery".to_owned(),
                    action: "ack".to_owned(),
                },
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };

        super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

        let ack_line = line_containing(rs_source, "delivery.ack()");
        let entry = entries
            .iter()
            .find(|entry| entry.id == "proof-map-case-0")
            .expect("ack discharge should get a generated anchor");
        assert_eq!(
            entry.rs_span.line, ack_line,
            "ack discharge must anchor to ack(), not an earlier same-binding method: {entry:?}"
        );
    }

    fn trace_kind_operations(span: kobo_ir::KoboSpan) -> Vec<ScenarioOp> {
        let binding = "delivery".to_owned();
        vec![
            ScenarioOp {
                span,
                kind: ScenarioOpKind::CreateObligation {
                    binding: binding.clone(),
                    type_name: "Delivery".to_owned(),
                    actions: vec!["ack".to_owned()],
                    template: Some(ScenarioLifecycleTemplate::inferred(
                        "queue_delivery",
                        "queue_delivery",
                    )),
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::MoveBinding {
                    binding: binding.clone(),
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::Transfer {
                    binding: binding.clone(),
                    callee: "handoff".to_owned(),
                    proven: true,
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::Discharge {
                    binding,
                    action: "ack".to_owned(),
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::ExternalBoundary {
                    crate_name: "boundary".to_owned(),
                    call_path: Some("boundary::call".to_owned()),
                    call_arguments: Vec::<ScenarioBoundaryCallArgument>::new(),
                    return_type: None,
                    call_shape: ScenarioExternalCallShape::FreeFunction,
                    policy: ScenarioBoundaryPolicy::Opaque,
                    reason: None,
                },
            },
            terminator(span, ScenarioCoreTerminatorKind::Return),
            terminator(span, ScenarioCoreTerminatorKind::ErrorExit),
            terminator(span, ScenarioCoreTerminatorKind::Panic),
            terminator(span, ScenarioCoreTerminatorKind::Await),
            terminator(span, ScenarioCoreTerminatorKind::OpaqueBoundary),
        ]
    }

    fn terminator(span: kobo_ir::KoboSpan, kind: ScenarioCoreTerminatorKind) -> ScenarioOp {
        ScenarioOp {
            span,
            kind: ScenarioOpKind::CoreTerminator {
                kind,
                boundary: None,
                policy: None,
                edges: Vec::new(),
            },
        }
    }

    fn single_discharge_program(
        file_id: FileId,
        target: &str,
        span: kobo_ir::KoboSpan,
    ) -> ScenarioProgram {
        ScenarioProgram {
            file_id,
            target: target.to_owned(),
            source_hash: "source".to_owned(),
            operations: vec![ScenarioOp {
                span,
                kind: ScenarioOpKind::Discharge {
                    binding: "delivery".to_owned(),
                    action: "ack".to_owned(),
                },
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        }
    }

    fn span_for(source: &str, function_needle: &str, event_needle: &str) -> kobo_ir::KoboSpan {
        let function_start = source
            .find(function_needle)
            .expect("function should exist in source");
        let event_start = source[function_start..]
            .find(event_needle)
            .map(|offset| function_start + offset)
            .expect("event should exist in source");
        kobo_ir::KoboSpan::new(
            event_start as u32,
            (event_start + event_needle.len()) as u32,
            FileId(1),
        )
    }

    fn line_containing(source: &str, needle: &str) -> usize {
        source
            .lines()
            .position(|line| line.contains(needle))
            .map(|index| index + 1)
            .expect("line should exist")
    }
}

fn ownership_tier_label(tier: OwnershipTier) -> &'static str {
    match tier {
        OwnershipTier::PlainOwned => "plain",
        OwnershipTier::BoxOwned => "box",
        OwnershipTier::RcShared => "rc",
        OwnershipTier::ArcShared => "arc",
        OwnershipTier::RcMutShared => "rc_refcell",
        OwnershipTier::ArcMutShared => "arc_rwlock",
        OwnershipTier::Scoped => "scoped_handle",
        OwnershipTier::Undecided => "plain",
    }
}

#[cfg(test)]
mod tests {
    use kobo_ir::{KoboSpan, OwnershipTier};

    use crate::lower::{LoweringSite, ResolvedAnchor, ResolvedAnchorMap};

    use super::{build_source_map_entries, wrap_source_map, RsSpan};

    #[test]
    fn source_map_supports_forward_and_reverse_lookup() {
        let mut anchors = ResolvedAnchorMap::default();
        anchors.insert(
            kobo_ir::KirNodeId(1),
            ResolvedAnchor {
                line: 2,
                column_start: 4,
                column_end: 9,
            },
        );
        let entries = build_source_map_entries(
            &[LoweringSite::new(
                kobo_ir::KirNodeId(1),
                "names",
                OwnershipTier::RcMutShared,
                KoboSpan::new(4, 9, kobo_ir::FileId(0)),
                2,
                "non-Copy shared binding",
            )],
            &anchors,
        );
        let source_map = wrap_source_map("src/main.kobo".as_ref(), "src/main.rs".as_ref(), entries);
        assert_eq!(source_map.generated_file(), "src/main.rs");
        assert_eq!(source_map.kobo_path(), "src/main.kobo");

        let kobo_span = source_map
            .lookup_kobo_span(RsSpan {
                line: 2,
                column_start: 0,
                column_end: 47,
            })
            .expect("mapped line should resolve");
        assert_eq!(kobo_span, KoboSpan::new(4, 9, kobo_ir::FileId(0)));

        let rs_spans = source_map.lookup_rs_spans(KoboSpan::new(4, 9, kobo_ir::FileId(0)));
        assert_eq!(rs_spans.len(), 1);
        assert_eq!(rs_spans[0].line, 2);
    }
}
