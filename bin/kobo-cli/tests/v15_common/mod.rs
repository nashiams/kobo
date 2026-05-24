#![allow(dead_code, unused_imports)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::v09_common::run_kobo_with_timeout;
pub use crate::v09_common::{
    assert_failure, assert_success, first_json, path_arg, s, CliOutput, TestProject,
};

const V15_TIMEOUT: Duration = Duration::from_secs(60);

pub fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V15_TIMEOUT)
}

pub fn queue_loop_source(
    scenario_name: &str,
    delivery_binding: &str,
    queue_binding: &str,
) -> String {
    format!(
        r#"
struct Queue {{}}

#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Queue {{
    fn recv(&self) -> Delivery {{ Delivery {{}} }}
}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let {queue_binding} = Queue {{}};
    loop {{
        let {delivery_binding} = {queue_binding}.recv();
        {delivery_binding}.ack();
    }}
}}
"#
    )
}

pub fn branch_leak_loop_source(scenario_name: &str) -> String {
    format!(
        r#"
struct Queue {{}}

#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Queue {{
    fn recv(&self) -> Delivery {{ Delivery {{}} }}
}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

fn choose() -> bool {{ true }}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    loop {{
        let delivery = queue.recv();
        if choose() {{
            delivery.ack();
        }}
    }}
}}
"#
    )
}

pub fn bounded_source(
    scenario_name: &str,
    completeness: &str,
    observed: u64,
    expected: u64,
    scheduler: Option<&str>,
    fault: Option<&str>,
    cancellation: Option<&str>,
) -> String {
    let mut fields = vec![
        format!(r#"histories = "{observed}""#),
        format!(r#"expected = "{expected}""#),
        format!(r#"completeness = "{completeness}""#),
    ];
    if let Some(scheduler) = scheduler {
        fields.push(format!(r#"scheduler = "{scheduler}""#));
    }
    if let Some(fault) = fault {
        fields.push(format!(r#"fault = "{fault}""#));
    }
    if let Some(cancellation) = cancellation {
        fields.push(format!(r#"cancellation = "{cancellation}""#));
    }
    format!(
        r#"
#[kobo::bounded({})]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let _unit = ();
}}
"#,
        fields.join(", ")
    )
}

pub fn emit_artifact(project: &TestProject, source: &str, scenario_name: &str) -> PathBuf {
    let source_file = project.main_file(source);
    let artifact_path = project.root.join(format!("{scenario_name}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s(scenario_name),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&output, "proof emit should succeed");
    artifact_path
}

pub fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

pub fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).expect("artifact should write");
}
