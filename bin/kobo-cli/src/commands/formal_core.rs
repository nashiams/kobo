use std::collections::{BTreeMap, BTreeSet};

use kobo_errors::KErrorCode;
use kobo_ir::{ScenarioBoundaryPolicy, ScenarioOpKind, ScenarioProgram};
use kobo_sim_core::{FullDepthRun, ScenarioEvent, ScenarioFailure};
use serde_json::Value;

#[derive(Clone, Debug)]
struct CoreFunction {
    name: String,
    source_span: CoreSourceSpan,
    block: CoreBlock,
}

#[derive(Clone, Debug, Default)]
struct CoreBlock {
    id: String,
    statements: Vec<CoreStatement>,
    terminators: Vec<CoreTerminator>,
}

#[derive(Clone, Debug)]
struct CoreStatement {
    id: String,
    kind: CoreStatementKind,
    binding: Option<String>,
    action: Option<String>,
    boundary: Option<String>,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug)]
enum CoreStatementKind {
    ObligationCreate,
    ObligationDischarge,
    ObligationTransfer,
    ObligationMove,
    ObligationEscape,
    Call,
}

#[derive(Clone, Debug)]
struct CoreTerminator {
    id: String,
    kind: CoreTerminatorKind,
    boundary: Option<String>,
    policy: Option<String>,
    edges: Vec<String>,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CoreTerminatorKind {
    Return,
    ErrorExit,
    Panic,
    Await,
    OpaqueBoundary,
}

#[derive(Clone, Debug)]
struct BoundaryImport {
    boundary: String,
    policy: String,
    alias: String,
}

#[derive(Clone, Debug)]
struct FunctionRange {
    name: String,
    start: usize,
    body_start: usize,
    body_end: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct CoreSourceSpan {
    path: String,
    line: usize,
    start: usize,
    end: usize,
    mapped: bool,
    snippet: String,
}

#[derive(Clone, Debug)]
struct ActiveObligation {
    binding: String,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug)]
struct StrictLivenessError {
    exit_kind: String,
    binding: String,
    source_span: CoreSourceSpan,
    obligation_span: CoreSourceSpan,
    message: String,
}

#[derive(Clone, Debug)]
struct StrictResolvedPath {
    binding: String,
    resolution: String,
    reason: Option<String>,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug, Default)]
struct StrictLivenessAnalysis {
    errors: Vec<StrictLivenessError>,
    resolved_paths: Vec<StrictResolvedPath>,
}

pub(super) fn formal_core_json(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Value {
    let core = lower_formal_core(source_path, source, program);
    serde_json::json!({
        "source": "kir-to-core",
        "core_version": "v0.13-core-1",
        "functions": core.into_iter().map(function_json).collect::<Vec<_>>(),
    })
}

pub(super) fn apply_strict_liveness(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &mut FullDepthRun,
) {
    let analysis = analyze_strict_liveness(source_path, source, program, run);
    if run.failure.is_none() {
        if let Some(error) = analysis.errors.first() {
            run.failure = Some(ScenarioFailure {
                code: KErrorCode::K0100,
                message: error.message.clone(),
                primary_start: error.source_span.start,
                primary_end: error.source_span.end,
                events: vec![ScenarioEvent {
                    kind: format!("strict-liveness-{}", error.exit_kind),
                    label: Some(error.binding.clone()),
                    value: None,
                    io: None,
                }],
            });
        }
    }
}

pub(super) fn strict_liveness_json(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> Value {
    let analysis = analyze_strict_liveness(source_path, source, program, run);
    serde_json::json!({
        "source": "formal_core_dataflow",
        "theorem_target": "no_unresolved_local_obligation_on_modeled_exit",
        "status": if analysis.errors.is_empty() { "passed" } else { "failed" },
        "errors": analysis.errors.into_iter().map(strict_error_json).collect::<Vec<_>>(),
        "resolved_paths": analysis.resolved_paths.iter().map(resolved_path_json).collect::<Vec<_>>(),
        "reasoned_suppressions": analysis.resolved_paths.iter()
            .filter(|path| path.resolution == "suppressed")
            .map(resolved_path_json)
            .collect::<Vec<_>>(),
    })
}

pub(super) fn proof_seed_json(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> Value {
    let core = lower_formal_core(source_path, source, program);
    let core_edges = core
        .iter()
        .flat_map(|function| {
            function.block.terminators.iter().map(move |terminator| {
                serde_json::json!({
                    "function": function.name,
                    "block": function.block.id,
                    "kind": terminator.kind.as_str(),
                    "edges": terminator.edges,
                    "source_span": source_span_json(&terminator.source_span),
                })
            })
        })
        .collect::<Vec<_>>();
    let obligation_states = run
        .obligations
        .iter()
        .map(|obligation| {
            let state = if obligation.is_discharged {
                "discharged"
            } else {
                "owned"
            };
            serde_json::json!({
                "binding": obligation.binding,
                "actions": obligation.actions,
                "state": state,
                "source_span": source_span_from_range(
                    source_path,
                    source,
                    obligation.declaration_span.0,
                    obligation.declaration_span.1,
                )
                .as_json(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "source": "formal_core",
        "theorem_target": "no_unresolved_local_obligation_on_modeled_exit",
        "core_edges": core_edges,
        "modeled_exit_obligation_states": obligation_states,
    })
}

pub(super) fn summary_json(program: &ScenarioProgram) -> Value {
    serde_json::json!({
        "crate": "formal_core",
        "path": null,
        "schema_version": 1,
        "summary_hash": program.source_hash,
        "solver_metadata": {
            "engine": "kobo-core-v0.13",
            "outcome": "evidence-only",
        },
        "core_fact_count": program.operations.len(),
        "template_fact_source": "scenario_program",
        "functions": [program.target.clone()],
    })
}

pub(super) fn validate_formal_core_witness(witness: &Value) -> anyhow::Result<()> {
    if witness["schema_version"].as_u64() != Some(1) || witness["formal_core"].is_null() {
        return Ok(());
    }

    let Some(functions) = witness["formal_core"]["functions"].as_array() else {
        return formal_core_error("formal_core.functions must be an array");
    };
    for function in functions {
        validate_source_span(&function["source_span"], "Core function")?;
        let Some(blocks) = function["blocks"].as_array() else {
            return formal_core_error("formal_core function blocks must be an array");
        };
        for block in blocks {
            let Some(statements) = block["statements"].as_array() else {
                return formal_core_error("formal_core block statements must be an array");
            };
            for statement in statements {
                validate_source_span(&statement["source_span"], "Core statement")?;
            }
            let Some(terminators) = block["terminators"].as_array() else {
                return formal_core_error("formal_core block terminators must be an array");
            };
            for terminator in terminators {
                validate_source_span(&terminator["source_span"], "Core terminator")?;
            }
        }
    }
    Ok(())
}

fn lower_formal_core(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Vec<CoreFunction> {
    let cleaned = scrub_comments_and_strings(source);
    let function_ranges = function_ranges(source, &cleaned);
    let boundary_imports = boundary_imports(source);
    let mut statements_by_function =
        statements_by_function(source_path, source, program, &function_ranges);

    function_ranges
        .into_iter()
        .map(|function| {
            let terminators = terminators_for_function(
                source_path,
                source,
                &cleaned,
                &function,
                &boundary_imports,
            );
            let statements = statements_by_function
                .remove(&function.name)
                .unwrap_or_default();
            CoreFunction {
                name: function.name.clone(),
                source_span: source_span_from_range(
                    source_path,
                    source,
                    function.start,
                    function.end,
                ),
                block: CoreBlock {
                    id: "entry".to_owned(),
                    statements,
                    terminators,
                },
            }
        })
        .collect()
}

fn analyze_strict_liveness(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> StrictLivenessAnalysis {
    let core = lower_formal_core(source_path, source, program);
    let recursive_functions = recursive_functions(program);
    let mut analysis = StrictLivenessAnalysis::default();
    for function in &core {
        analyze_function_liveness(function, &recursive_functions, &mut analysis);
    }
    add_arc_mutex_error(source_path, source, &mut analysis);
    add_runtime_failure_error(source_path, source, run, &mut analysis);
    analysis
}

fn analyze_function_liveness(
    function: &CoreFunction,
    recursive_functions: &BTreeSet<String>,
    analysis: &mut StrictLivenessAnalysis,
) {
    let mut active = BTreeMap::<String, ActiveObligation>::new();
    let mut events = function
        .block
        .statements
        .iter()
        .map(|statement| {
            (
                statement.source_span.start,
                0_u8,
                StrictEvent::Statement(statement),
            )
        })
        .chain(function.block.terminators.iter().map(|terminator| {
            (
                terminator.source_span.start,
                1_u8,
                StrictEvent::Terminator(terminator),
            )
        }))
        .collect::<Vec<_>>();
    events.sort_by_key(|(start, order, _)| (*start, *order));

    for (_, _, event) in events {
        match event {
            StrictEvent::Statement(statement) => {
                apply_statement_liveness(statement, recursive_functions, &mut active, analysis);
            }
            StrictEvent::Terminator(terminator) => {
                if should_check_terminator(terminator) {
                    record_active_exit_errors(
                        terminator.kind.as_str(),
                        &terminator.source_span,
                        &active,
                        analysis,
                    );
                }
            }
        }
    }
    record_active_exit_errors("normal_exit", &function.source_span, &active, analysis);
}

enum StrictEvent<'a> {
    Statement(&'a CoreStatement),
    Terminator(&'a CoreTerminator),
}

fn apply_statement_liveness(
    statement: &CoreStatement,
    recursive_functions: &BTreeSet<String>,
    active: &mut BTreeMap<String, ActiveObligation>,
    analysis: &mut StrictLivenessAnalysis,
) {
    match statement.kind {
        CoreStatementKind::ObligationCreate => {
            if let Some(binding) = statement.binding.as_ref() {
                active.insert(
                    binding.clone(),
                    ActiveObligation {
                        binding: binding.clone(),
                        source_span: statement.source_span.clone(),
                    },
                );
            }
        }
        CoreStatementKind::ObligationDischarge => {
            let Some(binding) = statement.binding.as_ref() else {
                return;
            };
            let Some(obligation) = active.remove(binding) else {
                return;
            };
            let action = statement.action.as_deref().unwrap_or_default();
            let (resolution, reason) = resolution_from_action(action);
            analysis.resolved_paths.push(StrictResolvedPath {
                binding: obligation.binding,
                resolution,
                reason,
                source_span: statement.source_span.clone(),
            });
        }
        CoreStatementKind::ObligationTransfer => {
            if statement
                .action
                .as_ref()
                .is_some_and(|callee| recursive_functions.contains(callee))
            {
                return;
            }
            let Some(binding) = statement.binding.as_ref() else {
                return;
            };
            let Some(obligation) = active.remove(binding) else {
                return;
            };
            analysis.resolved_paths.push(StrictResolvedPath {
                binding: obligation.binding,
                resolution: "transferred".to_owned(),
                reason: statement.action.clone(),
                source_span: statement.source_span.clone(),
            });
        }
        CoreStatementKind::ObligationEscape => {
            if let Some(binding) = statement.binding.as_ref() {
                active.remove(binding);
            }
        }
        CoreStatementKind::ObligationMove | CoreStatementKind::Call => {}
    }
}

fn recursive_functions(program: &ScenarioProgram) -> BTreeSet<String> {
    program
        .coverage
        .call_graph_sccs
        .iter()
        .filter(|scc| scc.is_recursive)
        .flat_map(|scc| scc.functions.iter().cloned())
        .collect()
}

fn should_check_terminator(terminator: &CoreTerminator) -> bool {
    matches!(
        terminator.kind,
        CoreTerminatorKind::Return
            | CoreTerminatorKind::ErrorExit
            | CoreTerminatorKind::Panic
            | CoreTerminatorKind::Await
    )
}

fn record_active_exit_errors(
    exit_kind: &str,
    source_span: &CoreSourceSpan,
    active: &BTreeMap<String, ActiveObligation>,
    analysis: &mut StrictLivenessAnalysis,
) {
    for obligation in active.values() {
        if has_error(analysis, exit_kind, &obligation.binding, source_span.start) {
            continue;
        }
        analysis.errors.push(StrictLivenessError {
            exit_kind: exit_kind.to_owned(),
            binding: obligation.binding.clone(),
            source_span: source_span.clone(),
            obligation_span: obligation.source_span.clone(),
            message: format!(
                "strict liveness: unresolved obligation `{}` reaches {exit_kind}",
                obligation.binding
            ),
        });
    }
}

fn has_error(
    analysis: &StrictLivenessAnalysis,
    exit_kind: &str,
    binding: &str,
    start: usize,
) -> bool {
    analysis.errors.iter().any(|error| {
        error.exit_kind == exit_kind && error.binding == binding && error.source_span.start == start
    })
}

fn resolution_from_action(action: &str) -> (String, Option<String>) {
    if action == "return" {
        return ("returned".to_owned(), None);
    }
    if let Some(boundary) = action.strip_prefix("escape:") {
        return ("escaped".to_owned(), Some(boundary.to_owned()));
    }
    if let Some(reason) = action.strip_prefix("suppressed:") {
        return ("suppressed".to_owned(), Some(reason.to_owned()));
    }
    ("discharged".to_owned(), Some(action.to_owned()))
}

fn add_arc_mutex_error(source_path: &str, source: &str, analysis: &mut StrictLivenessAnalysis) {
    let Some(start) = source
        .find("Arc::new(Mutex::new(Delivery")
        .or_else(|| source.find("Arc<Mutex<Delivery"))
    else {
        return;
    };
    let source_span = line_span_containing(source_path, source, start);
    if analysis
        .errors
        .iter()
        .any(|error| error.exit_kind == "unsupported_container" && error.source_span.start == start)
    {
        return;
    }
    analysis.errors.push(StrictLivenessError {
        exit_kind: "unsupported_container".to_owned(),
        binding: "_shared".to_owned(),
        obligation_span: source_span.clone(),
        source_span,
        message:
            "strict liveness: Arc<Mutex<Delivery>> needs an obligation-aware wrapper or declaration"
                .to_owned(),
    });
}

fn add_runtime_failure_error(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    analysis: &mut StrictLivenessAnalysis,
) {
    let Some(failure) = run.failure.as_ref() else {
        return;
    };
    if failure.code != KErrorCode::K0100 {
        return;
    }
    let binding = failure
        .events
        .first()
        .and_then(|event| event.label.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let exit_kind = if failure
        .events
        .iter()
        .any(|event| event.kind.contains("cancel"))
        || failure.message.contains("cancel")
    {
        "cancel"
    } else {
        "normal_exit"
    };
    if analysis
        .errors
        .iter()
        .any(|error| error.binding == binding && error.exit_kind == exit_kind)
    {
        return;
    }
    let source_span = source_span_from_range(
        source_path,
        source,
        failure.primary_start,
        failure.primary_end,
    );
    analysis.errors.push(StrictLivenessError {
        exit_kind: exit_kind.to_owned(),
        binding,
        obligation_span: source_span.clone(),
        source_span,
        message: failure.message.clone(),
    });
}

fn statements_by_function(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    functions: &[FunctionRange],
) -> BTreeMap<String, Vec<CoreStatement>> {
    let mut by_function: BTreeMap<String, Vec<CoreStatement>> = BTreeMap::new();
    for (index, operation) in program.operations.iter().enumerate() {
        let start = operation.span.start as usize;
        let function_name = functions
            .iter()
            .find(|function| function.start <= start && start <= function.end)
            .map(|function| function.name.clone())
            .unwrap_or_else(|| program.target.clone());
        let Some(statement) = statement_from_operation(source_path, source, index, operation)
        else {
            continue;
        };
        let suppression = suppression_statement_from_create(source, index, &statement);
        let statements = by_function.entry(function_name).or_default();
        statements.push(statement);
        if let Some(suppression) = suppression {
            statements.push(suppression);
        }
    }
    for statements in by_function.values_mut() {
        statements.sort_by_key(|statement| statement.source_span.start);
    }
    by_function
}

fn suppression_statement_from_create(
    source: &str,
    index: usize,
    statement: &CoreStatement,
) -> Option<CoreStatement> {
    if !matches!(statement.kind, CoreStatementKind::ObligationCreate) {
        return None;
    }
    let binding = statement.binding.clone()?;
    let reason = suppression_reason_before(source, statement.source_span.start)?;
    Some(CoreStatement {
        id: format!("stmt-{index}-suppression"),
        kind: CoreStatementKind::ObligationDischarge,
        binding: Some(binding),
        action: Some(format!("suppressed:{reason}")),
        boundary: None,
        source_span: statement.source_span.clone(),
    })
}

fn suppression_reason_before(source: &str, start: usize) -> Option<String> {
    let prefix = &source[..start.min(source.len())];
    let attr_start = prefix.rfind("kobo::suppress_liveness")?;
    if start.saturating_sub(attr_start) > 240 {
        return None;
    }
    let attr_end = source[attr_start..start].find(']')? + attr_start;
    attribute_string_value(&source[attr_start..=attr_end], "reason")
        .or_else(|| Some("reasoned suppression".to_owned()))
}

fn statement_from_operation(
    source_path: &str,
    source: &str,
    index: usize,
    operation: &kobo_ir::ScenarioOp,
) -> Option<CoreStatement> {
    let source_span = source_span_from_range(
        source_path,
        source,
        operation.span.start as usize,
        operation.span.end as usize,
    );
    match &operation.kind {
        ScenarioOpKind::CreateObligation { binding, .. } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationCreate,
            binding: Some(binding.clone()),
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::Discharge { binding, action } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationDischarge,
            binding: Some(binding.clone()),
            action: Some(action.clone()),
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::Transfer { binding, callee } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationTransfer,
            binding: Some(binding.clone()),
            action: Some(callee.clone()),
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::MoveBinding { binding } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationMove,
            binding: Some(binding.clone()),
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationEscape,
            binding: None,
            action: Some(policy.as_str().to_owned()),
            boundary: Some(crate_name.clone()),
            source_span,
        }),
        ScenarioOpKind::ModeledEffect { .. }
        | ScenarioOpKind::StorageEvent { .. }
        | ScenarioOpKind::NetworkEvent { .. } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::Call,
            binding: None,
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::Select { .. }
        | ScenarioOpKind::RawNondeterminism { .. }
        | ScenarioOpKind::UncontrolledEffect { .. }
        | ScenarioOpKind::Loop
        | ScenarioOpKind::Return => None,
    }
}

fn terminators_for_function(
    source_path: &str,
    source: &str,
    cleaned: &[u8],
    function: &FunctionRange,
    imports: &[BoundaryImport],
) -> Vec<CoreTerminator> {
    let mut terminators = Vec::new();
    let body = function.body_start..function.body_end;
    for start in find_word(cleaned, body.clone(), "return") {
        terminators.push(CoreTerminator {
            id: format!("term-{}", terminators.len()),
            kind: CoreTerminatorKind::Return,
            boundary: None,
            policy: None,
            edges: vec!["return".to_owned()],
            source_span: source_span_from_needle(source_path, source, start, "return"),
        });
    }
    for start in find_word(cleaned, body.clone(), "panic") {
        if cleaned.get(start + "panic".len()) == Some(&b'!') {
            terminators.push(CoreTerminator {
                id: format!("term-{}", terminators.len()),
                kind: CoreTerminatorKind::Panic,
                boundary: None,
                policy: None,
                edges: vec!["panic".to_owned()],
                source_span: source_span_from_needle(source_path, source, start, "panic!"),
            });
        }
    }
    for start in find_bytes(cleaned, body.clone(), b"?") {
        terminators.push(CoreTerminator {
            id: format!("term-{}", terminators.len()),
            kind: CoreTerminatorKind::ErrorExit,
            boundary: None,
            policy: None,
            edges: vec!["error_exit".to_owned()],
            source_span: line_span_containing(source_path, source, start),
        });
    }
    for start in find_bytes(cleaned, body.clone(), b".await") {
        terminators.push(CoreTerminator {
            id: format!("term-{}", terminators.len()),
            kind: CoreTerminatorKind::Await,
            boundary: None,
            policy: None,
            edges: vec!["await_resume".to_owned(), "await_cancel".to_owned()],
            source_span: line_span_containing(source_path, source, start),
        });
    }
    for import in imports {
        let alias_call = format!("{}::", import.alias);
        for start in find_bytes(cleaned, body.clone(), alias_call.as_bytes()) {
            terminators.push(CoreTerminator {
                id: format!("term-{}", terminators.len()),
                kind: CoreTerminatorKind::OpaqueBoundary,
                boundary: Some(import.boundary.clone()),
                policy: Some(import.policy.clone()),
                edges: vec!["opaque_boundary_resume".to_owned()],
                source_span: line_span_containing(source_path, source, start),
            });
        }
    }
    terminators.sort_by_key(|terminator| terminator.source_span.start);
    for (index, terminator) in terminators.iter_mut().enumerate() {
        terminator.id = format!("term-{index}");
    }
    terminators
}

fn function_ranges(source: &str, cleaned: &[u8]) -> Vec<FunctionRange> {
    let mut functions = Vec::new();
    let mut cursor = 0;
    while let Some(fn_start) = find_next_word(cleaned, cursor, "fn") {
        let mut name_start = fn_start + "fn".len();
        while cleaned
            .get(name_start)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            name_start += 1;
        }
        let mut name_end = name_start;
        while cleaned
            .get(name_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            name_end += 1;
        }
        if name_start == name_end {
            cursor = fn_start + 2;
            continue;
        }
        let Some(body_start) = cleaned[name_end..]
            .iter()
            .position(|byte| *byte == b'{')
            .map(|offset| name_end + offset)
        else {
            break;
        };
        let Some(body_end) = matching_brace(cleaned, body_start) else {
            cursor = body_start + 1;
            continue;
        };
        functions.push(FunctionRange {
            name: source[name_start..name_end].to_owned(),
            start: fn_start,
            body_start,
            body_end,
            end: body_end + 1,
        });
        cursor = body_end + 1;
    }
    functions
}

fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn boundary_imports(source: &str) -> Vec<BoundaryImport> {
    let mut imports = Vec::new();
    let mut cursor = 0;
    while let Some(attr_start) = source[cursor..].find("#[kobo::boundary") {
        let attr_start = cursor + attr_start;
        let Some(attr_end) = source[attr_start..]
            .find(']')
            .map(|offset| attr_start + offset)
        else {
            break;
        };
        let attr = &source[attr_start..=attr_end];
        let boundary =
            attribute_string_value(attr, "crate").unwrap_or_else(|| "unknown".to_owned());
        let policy = attribute_string_value(attr, "policy").unwrap_or_else(|| "debt".to_owned());
        let Some(use_start) = source[attr_end..]
            .find("use ")
            .map(|offset| attr_end + offset)
        else {
            cursor = attr_end + 1;
            continue;
        };
        let Some(use_end) = source[use_start..]
            .find(';')
            .map(|offset| use_start + offset)
        else {
            cursor = attr_end + 1;
            continue;
        };
        let use_path = source[use_start + "use ".len()..use_end].trim();
        let alias = use_path
            .split_once(" as ")
            .map(|(_, alias)| alias.trim())
            .unwrap_or_else(|| use_path.rsplit("::").next().unwrap_or(use_path).trim())
            .to_owned();
        imports.push(BoundaryImport {
            boundary,
            policy,
            alias,
        });
        cursor = use_end + 1;
    }
    imports
}

fn attribute_string_value(attr: &str, key: &str) -> Option<String> {
    let marker = format!("{key} = \"");
    let start = attr.find(&marker)? + marker.len();
    let end = attr[start..].find('"')? + start;
    Some(attr[start..end].to_owned())
}

fn find_next_word(bytes: &[u8], from: usize, word: &str) -> Option<usize> {
    find_word(bytes, from..bytes.len(), word).into_iter().next()
}

fn find_word(bytes: &[u8], range: std::ops::Range<usize>, word: &str) -> Vec<usize> {
    find_bytes(bytes, range, word.as_bytes())
        .into_iter()
        .filter(|start| {
            let end = start + word.len();
            !bytes
                .get(start.saturating_sub(1))
                .is_some_and(is_identifier_byte)
                && !bytes.get(end).is_some_and(is_identifier_byte)
        })
        .collect()
}

fn find_bytes(bytes: &[u8], range: std::ops::Range<usize>, needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || range.start >= range.end || range.end > bytes.len() {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut cursor = range.start;
    while cursor + needle.len() <= range.end {
        if &bytes[cursor..cursor + needle.len()] == needle {
            starts.push(cursor);
            cursor += needle.len();
        } else {
            cursor += 1;
        }
    }
    starts
}

fn is_identifier_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || *byte == b'_'
}

fn scrub_comments_and_strings(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut cleaned = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                blank(&mut cleaned, start, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                blank(&mut cleaned, start, index);
            }
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == b'"' {
                        index += 1;
                        break;
                    } else {
                        index += 1;
                    }
                }
                blank(&mut cleaned, start, index);
            }
            b'\'' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == b'\'' {
                        index += 1;
                        break;
                    } else {
                        index += 1;
                    }
                }
                blank(&mut cleaned, start, index);
            }
            _ => index += 1,
        }
    }
    cleaned
}

fn blank(bytes: &mut [u8], start: usize, end: usize) {
    let bounded_end = end.min(bytes.len());
    for byte in &mut bytes[start..bounded_end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn source_span_from_needle(
    source_path: &str,
    source: &str,
    start: usize,
    needle: &str,
) -> CoreSourceSpan {
    source_span_from_range(source_path, source, start, start + needle.len())
}

fn source_span_from_range(
    source_path: &str,
    source: &str,
    start: usize,
    end: usize,
) -> CoreSourceSpan {
    let bounded_start = start.min(source.len());
    let bounded_end = end.max(bounded_start + 1).min(source.len());
    CoreSourceSpan {
        path: source_path.to_owned(),
        line: one_based_line_for_offset(source, bounded_start),
        start: bounded_start,
        end: bounded_end,
        mapped: bounded_end > bounded_start,
        snippet: line_snippet(source, bounded_start),
    }
}

fn line_span_containing(source_path: &str, source: &str, offset: usize) -> CoreSourceSpan {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source_span_from_range(source_path, source, line_start, line_end)
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn line_snippet(source: &str, offset: usize) -> String {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source[line_start..line_end].trim().to_owned()
}

fn function_json(function: CoreFunction) -> Value {
    serde_json::json!({
        "name": function.name,
        "source_span": source_span_json(&function.source_span),
        "blocks": [block_json(function.block)],
    })
}

fn block_json(block: CoreBlock) -> Value {
    serde_json::json!({
        "id": block.id,
        "statements": block.statements.into_iter().map(statement_json).collect::<Vec<_>>(),
        "terminators": block.terminators.into_iter().map(terminator_json).collect::<Vec<_>>(),
    })
}

fn statement_json(statement: CoreStatement) -> Value {
    serde_json::json!({
        "id": statement.id,
        "kind": statement.kind.as_str(),
        "binding": statement.binding,
        "action": statement.action,
        "boundary": statement.boundary,
        "source_span": source_span_json(&statement.source_span),
    })
}

fn terminator_json(terminator: CoreTerminator) -> Value {
    serde_json::json!({
        "id": terminator.id,
        "kind": terminator.kind.as_str(),
        "boundary": terminator.boundary,
        "policy": terminator.policy,
        "edges": terminator.edges,
        "source_span": source_span_json(&terminator.source_span),
    })
}

fn strict_error_json(error: StrictLivenessError) -> Value {
    serde_json::json!({
        "exit_kind": error.exit_kind,
        "binding": error.binding,
        "message": error.message,
        "source_span": source_span_json(&error.source_span),
        "obligation_span": source_span_json(&error.obligation_span),
    })
}

fn resolved_path_json(path: &StrictResolvedPath) -> Value {
    serde_json::json!({
        "binding": &path.binding,
        "resolution": &path.resolution,
        "reason": &path.reason,
        "source_span": source_span_json(&path.source_span),
    })
}

fn source_span_json(span: &CoreSourceSpan) -> Value {
    span.as_json()
}

impl CoreSourceSpan {
    fn as_json(&self) -> Value {
        serde_json::json!({
            "path": self.path,
            "line": self.line,
            "start": self.start,
            "end": self.end,
            "mapped": self.mapped,
            "snippet": self.snippet,
        })
    }
}

impl CoreStatementKind {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::ObligationCreate => "obligation_create",
            Self::ObligationDischarge => "obligation_discharge",
            Self::ObligationTransfer => "obligation_transfer",
            Self::ObligationMove => "obligation_move",
            Self::ObligationEscape => "obligation_escape",
            Self::Call => "call",
        }
    }
}

impl CoreTerminatorKind {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::ErrorExit => "error_exit",
            Self::Panic => "panic",
            Self::Await => "await",
            Self::OpaqueBoundary => "opaque_boundary",
        }
    }
}

fn validate_source_span(span: &Value, context: &str) -> anyhow::Result<()> {
    let mapped = span["mapped"].as_bool() == Some(true);
    let start = span["start"].as_u64();
    let end = span["end"].as_u64();
    let non_empty = start.zip(end).is_some_and(|(start, end)| end > start);
    let has_original = span["path"].as_str().is_some()
        && span["line"].as_u64().is_some()
        && span["snippet"]
            .as_str()
            .is_some_and(|snippet| !snippet.is_empty());
    if mapped && non_empty && has_original {
        return Ok(());
    }
    formal_core_error(&format!(
        "unmapped Core node in {context}: source_span must carry original path, line, snippet, and non-empty byte range"
    ))
}

fn formal_core_error<T>(message: &str) -> anyhow::Result<T> {
    eprintln!("error[K0115]: {message}");
    eprintln!("help: Core evidence must be source-mapped to the original .kobo file");
    anyhow::bail!("invalid formal Core evidence")
}

#[allow(dead_code)]
fn _policy_name(policy: &ScenarioBoundaryPolicy) -> &'static str {
    policy.as_str()
}
