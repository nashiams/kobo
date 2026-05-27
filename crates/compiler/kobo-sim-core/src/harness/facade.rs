use std::collections::BTreeMap;

use kobo_ir::{
    ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioExternalCallShape,
    ScenarioOpKind, ScenarioProgram,
};

use crate::core::{ScenarioEvent, ScenarioOptions};
use crate::error::Result;

use super::events::event_print_statement;
use super::facade_manifest::{is_replay_owned_boundary, is_rust_identifier};
use super::record_boundary::{
    record_boundary_event_statement, record_boundary_runtime_support_source,
    BoundaryFacadeRecordCapture,
};

struct BoundaryFacade {
    functions: FunctionTree,
    methods: BTreeMap<String, BTreeMap<String, Vec<BoundaryFacadeEvent>>>,
}

enum FunctionTree {
    Module(BTreeMap<String, FunctionTree>),
    Function(Vec<BoundaryFacadeEvent>),
}

#[derive(Clone)]
struct BoundaryFacadeEvent {
    event: ScenarioEvent,
    record_capture: Option<BoundaryFacadeRecordCapture>,
}

enum BoundaryFacadeTarget {
    Function {
        module_path: Vec<String>,
        function_name: String,
    },
    Method {
        type_name: String,
        method_name: String,
    },
}

pub(super) fn external_boundary_support_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut crate_facades = BTreeMap::<String, BoundaryFacade>::new();
    for operation in &program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            call_path,
            call_arguments,
            return_type,
            call_shape,
            policy,
            reason,
            ..
        } = &operation.kind
        else {
            continue;
        };
        if !is_replay_owned_boundary(policy) || !is_rust_identifier(crate_name) {
            continue;
        }
        let target = boundary_facade_target(call_path.as_deref(), call_shape);
        let event = ScenarioEvent {
            kind: format!("boundary-{}", policy.as_str()),
            label: Some(boundary_event_label(
                crate_name,
                call_path.as_deref(),
                (operation.span.start as usize, operation.span.end as usize),
            )),
            value: Some(options.seed),
            io: None,
        };
        let record_capture = (*policy == ScenarioBoundaryPolicy::Record).then(|| {
            let span = (operation.span.start as usize, operation.span.end as usize);
            BoundaryFacadeRecordCapture {
                crate_name: crate_name.clone(),
                call_path: call_path.clone(),
                call_shape: call_shape.as_str().to_owned(),
                policy: policy.as_str().to_owned(),
                reason: reason.clone(),
                return_type: return_type.clone(),
                span,
                replay_key: boundary_event_label(crate_name, call_path.as_deref(), span),
                call_arguments: call_arguments.clone(),
            }
        });
        let facade_event = BoundaryFacadeEvent {
            event,
            record_capture,
        };
        match target {
            BoundaryFacadeTarget::Function {
                module_path,
                function_name,
            } if is_rust_identifier(&function_name)
                && module_path.iter().all(|module| is_rust_identifier(module)) =>
            {
                insert_function_event(
                    &mut crate_facades
                        .entry(crate_name.clone())
                        .or_default()
                        .functions,
                    &module_path,
                    function_name,
                    facade_event,
                );
            }
            BoundaryFacadeTarget::Method {
                type_name,
                method_name,
            } if is_rust_identifier(&type_name) && is_rust_identifier(&method_name) => {
                crate_facades
                    .entry(crate_name.clone())
                    .or_default()
                    .methods
                    .entry(type_name)
                    .or_default()
                    .entry(method_name)
                    .or_default()
                    .push(facade_event);
            }
            _ => {}
        }
    }

    let mut source = String::new();
    if crate_facades
        .values()
        .any(BoundaryFacade::has_record_capture)
    {
        source.push_str(record_boundary_runtime_support_source());
    }
    for (crate_name, facade) in crate_facades {
        source.push_str("mod ");
        source.push_str(&crate_name);
        source.push_str(" {\n");
        source.push_str("    pub struct __KoboBoundaryValue;\n");
        source.push_str(&function_tree_source(
            &facade.functions,
            1,
            vec![crate_name.clone()],
        )?);
        for (type_name, methods) in &facade.methods {
            for method_name in methods.keys() {
                source.push_str("    static ");
                source.push_str(&boundary_counter_name(type_name, method_name));
                source.push_str(
                    ": std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n",
                );
            }
        }
        for struct_name in facade_struct_names(&facade) {
            source.push_str("    pub struct ");
            source.push_str(&struct_name);
            source.push_str(";\n");
        }
        for (type_name, methods) in &facade.methods {
            source.push_str("    impl ");
            source.push_str(type_name);
            source.push_str(" {\n");
            for (method_name, events) in methods {
                let associated = generated_rust.contains(&format!("{type_name}::{method_name}("));
                if associated {
                    source.push_str("        pub fn ");
                    source.push_str(method_name);
                    source.push_str(&generic_argument_signature(events, true));
                    source.push_str(" -> ");
                    source.push_str(&facade_return_type_name(events, type_name));
                    source.push_str(" {\n");
                } else {
                    source.push_str("        pub fn ");
                    source.push_str(method_name);
                    source.push_str(&generic_argument_signature(events, false));
                    source.push_str(" -> ");
                    source.push_str(&facade_return_type_name(events, type_name));
                    source.push_str(" {\n");
                }
                source.push_str(&boundary_event_sequence_source(
                    &boundary_counter_name(type_name, method_name),
                    &format!("{crate_name}::{type_name}::{method_name}"),
                    events,
                )?);
                source.push_str("            ");
                source.push_str(&facade_return_type_name(events, type_name));
                source.push('\n');
                source.push_str("        }\n");
            }
            source.push_str("    }\n");
        }
        source.push_str("}\n");
    }
    Ok(source)
}

impl Default for BoundaryFacade {
    fn default() -> Self {
        Self {
            functions: FunctionTree::Module(BTreeMap::new()),
            methods: BTreeMap::new(),
        }
    }
}

impl BoundaryFacade {
    fn has_record_capture(&self) -> bool {
        self.functions.has_record_capture()
            || self.methods.values().any(|methods| {
                methods
                    .values()
                    .any(|events| events.iter().any(|event| event.record_capture.is_some()))
            })
    }
}

fn facade_struct_names(facade: &BoundaryFacade) -> Vec<String> {
    let mut names = facade.methods.keys().cloned().collect::<Vec<_>>();
    for methods in facade.methods.values() {
        for events in methods.values() {
            if let Some(return_type) = facade_return_type_path(events) {
                if let Some(name) = boundary_type_leaf(&return_type) {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

impl FunctionTree {
    fn has_record_capture(&self) -> bool {
        match self {
            Self::Module(children) => children.values().any(Self::has_record_capture),
            Self::Function(events) => events.iter().any(|event| event.record_capture.is_some()),
        }
    }
}

fn boundary_facade_target(
    call_path: Option<&str>,
    call_shape: &ScenarioExternalCallShape,
) -> BoundaryFacadeTarget {
    let Some(call_path) = call_path else {
        return BoundaryFacadeTarget::Method {
            type_name: "Client".to_owned(),
            method_name: "new".to_owned(),
        };
    };
    let segments = call_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if matches!(call_shape, ScenarioExternalCallShape::FreeFunction) {
        return BoundaryFacadeTarget::Function {
            module_path: segments
                .iter()
                .skip(1)
                .take(segments.len().saturating_sub(2))
                .map(|segment| (*segment).to_owned())
                .collect(),
            function_name: segments.last().unwrap_or(&"boundary").to_string(),
        };
    }
    if segments.len() < 2 {
        return BoundaryFacadeTarget::Function {
            module_path: Vec::new(),
            function_name: segments.last().unwrap_or(&"boundary").to_string(),
        };
    }
    BoundaryFacadeTarget::Method {
        type_name: segments[segments.len() - 2].to_owned(),
        method_name: segments[segments.len() - 1].to_owned(),
    }
}

fn insert_function_event(
    tree: &mut FunctionTree,
    module_path: &[String],
    function_name: String,
    event: BoundaryFacadeEvent,
) {
    let FunctionTree::Module(children) = tree else {
        return;
    };
    let Some((module_name, remaining_modules)) = module_path.split_first() else {
        let function = children
            .entry(function_name)
            .or_insert_with(|| FunctionTree::Function(Vec::new()));
        if let FunctionTree::Function(events) = function {
            events.push(event);
        }
        return;
    };
    let module = children
        .entry(module_name.clone())
        .or_insert_with(|| FunctionTree::Module(BTreeMap::new()));
    insert_function_event(module, remaining_modules, function_name, event);
}

fn function_tree_source(
    tree: &FunctionTree,
    indent_level: usize,
    module_path: Vec<String>,
) -> Result<String> {
    let mut source = String::new();
    let FunctionTree::Module(children) = tree else {
        return Ok(source);
    };
    for (name, child) in children {
        match child {
            FunctionTree::Module(_) => {
                let indent = "    ".repeat(indent_level);
                source.push_str(&indent);
                source.push_str("pub mod ");
                source.push_str(name);
                source.push_str(" {\n");
                let mut child_path = module_path.clone();
                child_path.push(name.clone());
                source.push_str(&function_tree_source(child, indent_level + 1, child_path)?);
                source.push_str(&indent);
                source.push_str("}\n");
            }
            FunctionTree::Function(events) => {
                let indent = "    ".repeat(indent_level);
                let counter_name = boundary_counter_name("fn", name);
                source.push_str(&indent);
                source.push_str("static ");
                source.push_str(&counter_name);
                source.push_str(
                    ": std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n",
                );
                source.push_str(&indent);
                source.push_str("pub fn ");
                source.push_str(name);
                source.push_str(&generic_argument_signature(events, true));
                source.push_str(" -> crate::");
                source.push_str(module_path.first().map(String::as_str).unwrap_or("self"));
                source.push_str("::__KoboBoundaryValue {\n");
                let mut function_path = module_path.clone();
                function_path.push(name.clone());
                source.push_str(&boundary_event_sequence_source(
                    &counter_name,
                    &function_path.join("::"),
                    events,
                )?);
                source.push_str(&indent);
                source.push_str("    crate::");
                source.push_str(module_path.first().map(String::as_str).unwrap_or("self"));
                source.push_str("::__KoboBoundaryValue\n");
                source.push_str(&indent);
                source.push_str("}\n");
            }
        }
    }
    Ok(source)
}

fn generic_argument_signature(events: &[BoundaryFacadeEvent], no_self: bool) -> String {
    let arguments = facade_call_arguments(events);
    let generics = if arguments.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            arguments
                .iter()
                .map(|argument| format!("__KoboArg{}", argument.index))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut parameters = Vec::new();
    if !no_self {
        parameters.push("self".to_owned());
    }
    parameters.extend(
        arguments
            .iter()
            .map(|argument| format!("__kobo_arg{}: __KoboArg{}", argument.index, argument.index)),
    );
    format!("{generics}({})", parameters.join(", "))
}

fn facade_call_arguments(events: &[BoundaryFacadeEvent]) -> Vec<ScenarioBoundaryCallArgument> {
    events
        .iter()
        .find_map(|event| event.record_capture.as_ref())
        .map(|capture| capture.call_arguments.clone())
        .unwrap_or_default()
}

fn facade_return_type_name(events: &[BoundaryFacadeEvent], fallback: &str) -> String {
    facade_return_type_path(events)
        .and_then(|path| boundary_type_leaf(&path))
        .unwrap_or_else(|| fallback.to_owned())
}

fn facade_return_type_path(events: &[BoundaryFacadeEvent]) -> Option<String> {
    events
        .iter()
        .find_map(|event| event.record_capture.as_ref())
        .and_then(|capture| capture.return_type.clone())
}

fn boundary_type_leaf(path: &str) -> Option<String> {
    path.split("::")
        .filter(|segment| !segment.is_empty())
        .last()
        .map(str::to_owned)
}

fn boundary_counter_name(type_name: &str, method_name: &str) -> String {
    format!(
        "__KOBO_{}_{}_BOUNDARY_INDEX",
        type_name.to_ascii_uppercase(),
        method_name.to_ascii_uppercase()
    )
}

fn boundary_call_arguments_source(events: &[BoundaryFacadeEvent]) -> String {
    let arguments = facade_call_arguments(events);
    if arguments.is_empty() {
        return "            let __kobo_call_arguments: Vec<(usize, &'static str, String, String)> = Vec::new();\n".to_owned();
    }
    let mut source =
        "            let __kobo_call_arguments: Vec<(usize, &'static str, String, String)> = vec![\n"
            .to_owned();
    for argument in arguments {
        source.push_str("                (");
        source.push_str(&argument.index.to_string());
        source.push_str(", ");
        source.push_str(&format!("{:?}", argument.source));
        source.push_str(", std::any::type_name::<__KoboArg");
        source.push_str(&argument.index.to_string());
        source.push_str(">().to_owned(), std::mem::size_of_val(&__kobo_arg");
        source.push_str(&argument.index.to_string());
        source.push_str(").to_string()),\n");
    }
    source.push_str("            ];\n");
    source
}

fn boundary_facade_event_statement(event: &BoundaryFacadeEvent) -> Result<String> {
    if let Some(capture) = event.record_capture.as_ref() {
        return Ok(record_boundary_event_statement(&event.event, capture));
    }
    event_print_statement(&event.event)
}

fn boundary_event_sequence_source(
    counter_name: &str,
    label: &str,
    events: &[BoundaryFacadeEvent],
) -> Result<String> {
    let mut source = String::new();
    source.push_str(&boundary_call_arguments_source(events));
    source.push_str("            let __kobo_index = ");
    source.push_str(counter_name);
    source.push_str(".fetch_add(1, std::sync::atomic::Ordering::SeqCst);\n");
    source.push_str("            match __kobo_index {\n");
    for (index, event) in events.iter().enumerate() {
        source.push_str("                ");
        source.push_str(&index.to_string());
        source.push_str(" => { ");
        source.push_str(&boundary_facade_event_statement(event)?);
        source.push_str(" }\n");
    }
    let overflow = BoundaryFacadeEvent {
        event: ScenarioEvent {
            kind: "boundary-overflow".to_owned(),
            label: Some(label.to_owned()),
            value: None,
            io: None,
        },
        record_capture: None,
    };
    source.push_str("                _ => { ");
    source.push_str(&boundary_facade_event_statement(&overflow)?);
    source.push_str(" }\n");
    source.push_str("            }\n");
    Ok(source)
}

fn boundary_event_label(crate_name: &str, call_path: Option<&str>, span: (usize, usize)) -> String {
    format!("{}@{}..{}", call_path.unwrap_or(crate_name), span.0, span.1)
}
