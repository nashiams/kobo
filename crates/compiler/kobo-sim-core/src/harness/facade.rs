use std::collections::BTreeMap;

mod render;

use kobo_ir::{ScenarioBoundaryPolicy, ScenarioExternalCallShape, ScenarioOpKind, ScenarioProgram};

use crate::core::{ScenarioEvent, ScenarioOptions};
use crate::error::Result;

use super::facade_manifest::{is_replay_owned_boundary, is_rust_identifier};
use super::record_boundary::BoundaryFacadeRecordCapture;

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
    let crate_facades = collect_boundary_facades(program, options);
    render::render_boundary_facades(crate_facades, generated_rust)
}

fn collect_boundary_facades(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> BTreeMap<String, BoundaryFacade> {
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
    crate_facades
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

fn boundary_event_label(crate_name: &str, call_path: Option<&str>, span: (usize, usize)) -> String {
    format!("{}@{}..{}", call_path.unwrap_or(crate_name), span.0, span.1)
}
