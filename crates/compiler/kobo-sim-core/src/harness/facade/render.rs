use kobo_ir::ScenarioBoundaryCallArgument;

use crate::core::ScenarioEvent;
use crate::error::Result;

use super::super::events::event_print_statement;
use super::super::record_boundary::{
    record_boundary_event_statement, record_boundary_runtime_support_source,
};
use super::{BoundaryFacade, BoundaryFacadeEvent, FunctionTree};

pub(super) fn render_boundary_facades(
    crate_facades: std::collections::BTreeMap<String, BoundaryFacade>,
    generated_rust: &str,
) -> Result<String> {
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
        render_method_counters(&mut source, &facade);
        for struct_name in facade_struct_names(&facade) {
            source.push_str("    pub struct ");
            source.push_str(&struct_name);
            source.push_str(";\n");
        }
        render_method_impls(&mut source, &crate_name, &facade, generated_rust)?;
        source.push_str("}\n");
    }
    Ok(source)
}

fn render_method_counters(source: &mut String, facade: &BoundaryFacade) {
    for (type_name, methods) in &facade.methods {
        for method_name in methods.keys() {
            source.push_str("    static ");
            source.push_str(&boundary_counter_name(type_name, method_name));
            source.push_str(
                ": std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n",
            );
        }
    }
}

fn render_method_impls(
    source: &mut String,
    crate_name: &str,
    facade: &BoundaryFacade,
    generated_rust: &str,
) -> Result<()> {
    for (type_name, methods) in &facade.methods {
        source.push_str("    impl ");
        source.push_str(type_name);
        source.push_str(" {\n");
        for (method_name, events) in methods {
            render_method(
                source,
                crate_name,
                type_name,
                method_name,
                events,
                generated_rust,
            )?;
        }
        source.push_str("    }\n");
    }
    Ok(())
}

fn render_method(
    source: &mut String,
    crate_name: &str,
    type_name: &str,
    method_name: &str,
    events: &[BoundaryFacadeEvent],
    generated_rust: &str,
) -> Result<()> {
    let associated = generated_rust.contains(&format!("{type_name}::{method_name}("));
    source.push_str("        pub fn ");
    source.push_str(method_name);
    source.push_str(&generic_argument_signature(events, associated));
    source.push_str(" -> ");
    source.push_str(&facade_return_type_name(events, type_name));
    source.push_str(" {\n");
    source.push_str(&boundary_event_sequence_source(
        &boundary_counter_name(type_name, method_name),
        &format!("{crate_name}::{type_name}::{method_name}"),
        events,
    )?);
    source.push_str("            ");
    source.push_str(&facade_return_type_name(events, type_name));
    source.push('\n');
    source.push_str("        }\n");
    Ok(())
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
                source.push_str(&module_source(name, child, indent_level, &module_path)?);
            }
            FunctionTree::Function(events) => {
                source.push_str(&function_source(name, events, indent_level, &module_path)?);
            }
        }
    }
    Ok(source)
}

fn module_source(
    name: &str,
    child: &FunctionTree,
    indent_level: usize,
    module_path: &[String],
) -> Result<String> {
    let indent = "    ".repeat(indent_level);
    let mut child_path = module_path.to_vec();
    child_path.push(name.to_owned());

    let mut source = String::new();
    source.push_str(&indent);
    source.push_str("pub mod ");
    source.push_str(name);
    source.push_str(" {\n");
    source.push_str(&function_tree_source(child, indent_level + 1, child_path)?);
    source.push_str(&indent);
    source.push_str("}\n");
    Ok(source)
}

fn function_source(
    name: &str,
    events: &[BoundaryFacadeEvent],
    indent_level: usize,
    module_path: &[String],
) -> Result<String> {
    let indent = "    ".repeat(indent_level);
    let counter_name = boundary_counter_name("fn", name);
    let crate_name = module_path.first().map(String::as_str).unwrap_or("self");
    let mut function_path = module_path.to_vec();
    function_path.push(name.to_owned());

    let mut source = String::new();
    source.push_str(&indent);
    source.push_str("static ");
    source.push_str(&counter_name);
    source.push_str(": std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n");
    source.push_str(&indent);
    source.push_str("pub fn ");
    source.push_str(name);
    source.push_str(&generic_argument_signature(events, true));
    source.push_str(" -> crate::");
    source.push_str(crate_name);
    source.push_str("::__KoboBoundaryValue {\n");
    source.push_str(&boundary_event_sequence_source(
        &counter_name,
        &function_path.join("::"),
        events,
    )?);
    source.push_str(&indent);
    source.push_str("    crate::");
    source.push_str(crate_name);
    source.push_str("::__KoboBoundaryValue\n");
    source.push_str(&indent);
    source.push_str("}\n");
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
