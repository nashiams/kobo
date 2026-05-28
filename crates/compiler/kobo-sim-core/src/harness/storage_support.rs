use kobo_ir::{ScenarioOpKind, ScenarioProgram};

use crate::core::ScenarioOptions;
use crate::error::Result;

use super::events::event_print_statements;

pub(super) fn storage_support_source(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut methods = Vec::new();
    for operation in &program.operations {
        if let ScenarioOpKind::StorageEvent { action } = &operation.kind {
            if methods.iter().any(|existing| existing == action) {
                continue;
            }
            methods.push(action.clone());
        }
    }
    if methods.is_empty() {
        methods.push("write".to_owned());
        methods.push("crash_after_write".to_owned());
    }
    let mut source = String::from(
        r#"
fn __kobo_storage_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("kobo-generated-storage-{}", std::process::id()))
}

fn __kobo_storage_journal_path() -> std::path::PathBuf {
    let root = __kobo_storage_root();
    let _ = std::fs::create_dir_all(&root);
    root.join("journal.log")
}

fn __kobo_storage_write_record() {
    let path = __kobo_storage_journal_path();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_all(&mut file, b"kobo-storage-record\n");
    }
}

fn __kobo_storage_commit_record() {
    let path = __kobo_storage_journal_path();
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.sync_all();
    }
}

fn __kobo_storage_recover_record() {
    let path = __kobo_storage_journal_path();
    let _ = std::fs::read(path);
}

"#,
    );
    source.push_str("impl __KoboWardStorage {\n");
    for method in methods {
        source.push_str("    fn ");
        source.push_str(&method);
        if crate::storage::method_takes_value(&method) {
            source.push_str("<T>(&self, _value: T) {\n        ");
        } else {
            source.push_str("(&self) {\n        ");
        }
        source.push_str(&storage_runtime_statement(&method));
        source.push_str("\n        ");
        source.push_str(&event_print_statements(
            &crate::storage::events_for_action(&method, options.seed),
        )?);
        source.push_str("\n    }\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn storage_runtime_statement(method: &str) -> &'static str {
    match normalized_storage_method(method).as_str() {
        "write" | "append" | "journal" => "__kobo_storage_write_record();",
        "commit" | "flush" | "fsync" => "__kobo_storage_commit_record();",
        "recover" | "replay" => "__kobo_storage_recover_record();",
        _ => "",
    }
}

fn normalized_storage_method(method: &str) -> String {
    method
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}
