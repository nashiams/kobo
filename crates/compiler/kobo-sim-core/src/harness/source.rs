use std::path::Path;

use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{ScenarioEvent, ScenarioOptions};
use crate::error::{Result, SimCoreError};

use super::events::{event_marker, event_print_statement, event_print_statements};
use super::facade::external_boundary_support_source;
use super::failures::{boundary_label, modeled_boundary_events, terminal_failure_events};
use super::network_support::network_support_source;
use super::storage_support::storage_support_source;
use super::tokio_support::tokio_support_source;

pub(super) fn harness_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    loom_checkpoint_path: Option<&Path>,
) -> Result<String> {
    if has_event_marker(generated_rust) {
        return Ok(generated_rust.to_owned());
    }

    instrument_generated_rust(program, generated_rust, options, loom_checkpoint_path)
}

fn has_event_marker(source: &str) -> bool {
    source.contains(&event_marker()) && source.contains("fn main")
}

fn instrument_generated_rust(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    loom_checkpoint_path: Option<&Path>,
) -> Result<String> {
    let mut source = String::new();
    source.push_str(&harness_support_source(program, generated_rust, options)?);
    source.push_str(&strip_harness_only_attrs(generated_rust));
    if !source.ends_with('\n') {
        source.push('\n');
    }

    for operation in &program.operations {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            let events = modeled_boundary_events(boundary, options);
            source = inject_modeled_boundary_event(source, boundary, &events)?;
        }
    }

    let final_events = terminal_failure_events(program, options);
    source.push_str(&main_wrapper_source(
        &program.target,
        &final_events,
        options,
        loom_checkpoint_path,
        target_is_async(generated_rust, &program.target),
    )?);
    Ok(source)
}

fn strip_harness_only_attrs(source: &str) -> String {
    let mut output = String::new();
    let mut skipping_kobo_attr = false;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if skipping_kobo_attr {
            if trimmed.ends_with(']') {
                skipping_kobo_attr = false;
            }
            continue;
        }
        if is_harness_only_kobo_attr(trimmed) {
            skipping_kobo_attr = !trimmed.ends_with(']');
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn is_harness_only_kobo_attr(trimmed_line: &str) -> bool {
    trimmed_line.starts_with("#[kobo::boundary")
        || trimmed_line.starts_with("#[kobo::record")
        || trimmed_line.starts_with("#[kobo::activity")
}

fn harness_support_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut source = String::from(
        r#"
#[allow(non_camel_case_types)]
struct __KoboWardTime;
#[allow(non_camel_case_types)]
struct __KoboWardRandom;
#[allow(non_camel_case_types)]
struct __KoboWardStorage;
#[allow(non_camel_case_types)]
struct __KoboWardNetwork;
#[allow(non_camel_case_types)]
struct __KoboWard {
    time: __KoboWardTime,
    random: __KoboWardRandom,
    storage: __KoboWardStorage,
    network: __KoboWardNetwork,
}
#[allow(non_upper_case_globals)]
static ward: __KoboWard = __KoboWard {
    time: __KoboWardTime,
    random: __KoboWardRandom,
    storage: __KoboWardStorage,
    network: __KoboWardNetwork,
};
impl __KoboWard {
    fn task(&self) {}
}
impl __KoboWardTime {
    fn now(&self) -> u64 { 0 }
}
impl __KoboWardRandom {
    fn u64(&self) -> u64 { 0 }
    fn next_u64(&self) -> u64 { 0 }
}

fn __kobo_block_on<F: std::future::Future>(future: F) -> F::Output {
    fn clone(_: *const ()) -> std::task::RawWaker {
        raw_waker()
    }
    fn wake(_: *const ()) {}
    fn wake_by_ref(_: *const ()) {}
    fn drop(_: *const ()) {}
    fn raw_waker() -> std::task::RawWaker {
        std::task::RawWaker::new(
            std::ptr::null(),
            &std::task::RawWakerVTable::new(clone, wake, wake_by_ref, drop),
        )
    }

    let waker = unsafe { std::task::Waker::from_raw(raw_waker()) };
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => return output,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}
"#,
    );

    source.push_str(&storage_support_source(program, options)?);
    source.push_str(&network_support_source(program, options)?);
    source.push_str(&external_boundary_support_source(
        program,
        generated_rust,
        options,
    )?);
    source.push_str(&tokio_support_source(program, options)?);

    let mut impls = Vec::new();
    for operation in &program.operations {
        if let ScenarioOpKind::CreateObligation {
            type_name, actions, ..
        } = &operation.kind
        {
            if impls.iter().any(|existing| existing == type_name) {
                continue;
            }
            impls.push(type_name.clone());
            if !generated_rust_defines_type(generated_rust, type_name) {
                source.push_str("struct ");
                source.push_str(type_name);
                source.push_str(";\n");
            }
            source.push_str("impl ");
            source.push_str(type_name);
            source.push_str(" {\n");
            for action in actions {
                source.push_str("    fn ");
                source.push_str(&rust_method_name(action));
                source.push_str("(self) {}\n");
            }
            source.push_str("}\n");
        }
    }
    Ok(source)
}

fn generated_rust_defines_type(source: &str, type_name: &str) -> bool {
    let struct_pattern = format!("struct {type_name}");
    let enum_pattern = format!("enum {type_name}");
    let type_pattern = format!("type {type_name}");
    source.contains(&struct_pattern)
        || source.contains(&enum_pattern)
        || source.contains(&type_pattern)
}

fn rust_method_name(action: &str) -> String {
    match action {
        "await" => "r#await".to_owned(),
        _ => action.replace('-', "_"),
    }
}

fn inject_modeled_boundary_event(
    source: String,
    boundary: &ScenarioModeledBoundary,
    events: &[ScenarioEvent],
) -> Result<String> {
    let print = event_print_statements(events)?;
    let replacements: &[(&str, &str)] = match boundary {
        ScenarioModeledBoundary::WardTask | ScenarioModeledBoundary::WardTaskLocal => {
            &[("ward.task();", "ward.task();")]
        }
        ScenarioModeledBoundary::WardTime => &[("ward.time.now()", "ward.time.now()")],
        ScenarioModeledBoundary::WardRandom => &[
            ("ward.random.u64()", "ward.random.u64()"),
            ("ward.random.next_u64()", "ward.random.next_u64()"),
        ],
    };
    for (needle, replacement) in replacements {
        if source.contains(needle) {
            return Ok(source.replacen(needle, &instrumented_expression(replacement, &print), 1));
        }
    }
    if boundary == &ScenarioModeledBoundary::WardTask {
        for needle in ["tokio::spawn(async {})", "tokio :: spawn(async {})"] {
            if source.contains(needle) {
                return Ok(source.replacen(
                    needle,
                    &format!("{{\n        {print}\n        {needle}\n    }}"),
                    1,
                ));
            }
        }
        for needle in [
            "tokio::spawn(async move {",
            "tokio :: spawn(async move {",
            "tokio::spawn(async {",
            "tokio :: spawn(async {",
        ] {
            if source.contains(needle) {
                return Ok(source.replacen(needle, &format!("{print}\n    {needle}"), 1));
            }
        }
    }
    Err(SimCoreError::ModeledBoundaryMissing {
        boundary: boundary_label(boundary),
    })
}

fn instrumented_expression(expression: &str, print: &str) -> String {
    if expression.ends_with(';') {
        format!("{expression}\n    {print}")
    } else {
        format!("{{ let __kobo_value = {expression}; {print} __kobo_value }}")
    }
}

fn main_wrapper_source(
    target: &str,
    final_events: &[ScenarioEvent],
    options: &ScenarioOptions,
    loom_checkpoint_path: Option<&Path>,
    target_is_async: bool,
) -> Result<String> {
    let mut source = if options.profile == "sync" {
        let mut source = String::from("\nfn main() {\n");
        if options.scheduler == crate::SchedulerPolicy::Exhaustive
            || options.loom_max_branches.is_some()
            || loom_checkpoint_path.is_some()
        {
            source.push_str("    let mut __kobo_loom = loom::model::Builder::new();\n");
            if let Some(max_branches) = options.loom_max_branches {
                source.push_str(&format!(
                    "    __kobo_loom.max_branches = {max_branches}_usize;\n"
                ));
            }
            if let Some(checkpoint_path) = loom_checkpoint_path {
                source.push_str("    __kobo_loom.checkpoint_file(");
                source.push_str(&format!("{:?}", checkpoint_path.display().to_string()));
                source.push_str(");\n");
                source.push_str("    __kobo_loom.checkpoint_interval = 1;\n");
            }
            source.push_str("    __kobo_loom.check(|| {\n");
        } else {
            source.push_str("    loom::model(|| {\n");
        }
        source
    } else {
        String::from("\nfn main() {\n")
    };
    source.push_str("    ");
    if options.profile == "sync" {
        source.push_str("    ");
    }
    if target_is_async {
        source.push_str("__kobo_block_on(");
        source.push_str(target);
        source.push_str("());\n");
    } else {
        source.push_str(target);
        source.push_str("();\n");
    }
    for event in final_events {
        source.push_str("    ");
        if options.profile == "sync" {
            source.push_str("    ");
        }
        source.push_str(&event_print_statement(event)?);
        source.push('\n');
    }
    if options.profile == "sync" {
        source.push_str("    });\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn target_is_async(source: &str, target: &str) -> bool {
    source.contains(&format!("async fn {target}"))
}
