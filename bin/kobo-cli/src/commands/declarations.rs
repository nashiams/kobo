use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) struct DeclarationFacts {
    pub(super) path: PathBuf,
    pub(super) version: String,
    pub(super) schema_version: i64,
    pub(super) hash: String,
    pub(super) types: Vec<String>,
    pub(super) functions: Vec<String>,
    pub(super) function_facts: Vec<DeclarationFunctionFact>,
    pub(super) effects: Vec<String>,
    pub(super) adapters: Vec<String>,
    pub(super) activity_facts: Vec<DeclarationActivityFact>,
}

#[derive(Clone)]
pub(super) struct DeclarationFunctionFact {
    pub(super) path: String,
    pub(super) effects: Vec<String>,
    pub(super) simulation: Option<String>,
    pub(super) determinism: Option<String>,
}

#[derive(Clone)]
pub(super) struct DeclarationActivityFact {
    pub(super) path: String,
    pub(super) retry: Option<String>,
    pub(super) idempotency: Option<String>,
    pub(super) result: Option<String>,
    pub(super) compensation: Option<String>,
}

pub(super) struct DeclarationError {
    pub(super) path: PathBuf,
    pub(super) key: &'static str,
    pub(super) message: String,
}

pub(super) enum DeclarationLookup {
    Valid(DeclarationFacts),
    Missing,
    Invalid(DeclarationError),
}

pub(super) fn load_declaration(
    file: &Path,
    crate_name: &str,
    expected_version: Option<&str>,
) -> DeclarationLookup {
    let Some(search_root) = file.parent() else {
        return DeclarationLookup::Missing;
    };
    for ancestor in search_root.ancestors() {
        for candidate in declaration_candidates(ancestor, crate_name) {
            if candidate.is_file() {
                return parse_declaration_file(&candidate, crate_name, expected_version);
            }
        }
    }
    DeclarationLookup::Missing
}

pub(super) fn declaration_path_for(file: &Path, crate_name: &str) -> Option<PathBuf> {
    let Some(search_root) = file.parent() else {
        return None;
    };
    for ancestor in search_root.ancestors() {
        for candidate in declaration_candidates(ancestor, crate_name) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub(super) fn declaration_candidates(root: &Path, crate_name: &str) -> Vec<PathBuf> {
    vec![
        root.join(format!("{crate_name}.kobo.d.toml")),
        root.join("crate.kobo.d.toml"),
        root.join("kobo.d.toml"),
        root.join("kobo.d").join(format!("{crate_name}.toml")),
        root.join("declarations")
            .join(format!("{crate_name}.kobo.d.toml")),
        root.join(".kobo")
            .join("declarations")
            .join(format!("{crate_name}.kobo.d.toml")),
    ]
}

pub(super) fn parse_declaration_file(
    path: &Path,
    crate_name: &str,
    expected_version: Option<&str>,
) -> DeclarationLookup {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "file",
                message: error.to_string(),
            })
        }
    };
    let parsed = match source.parse::<toml::Value>() {
        Ok(parsed) => parsed,
        Err(error) => {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "crate.name",
                message: error.to_string(),
            })
        }
    };
    let schema_version = parsed
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if schema_version != 0 {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "schema_version",
            message: "only declaration schema_version 0 is supported".to_owned(),
        });
    }
    let Some(crate_table) = parsed.get("crate").and_then(toml::Value::as_table) else {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "crate",
            message: "missing [crate] table".to_owned(),
        });
    };
    if crate_table.get("name").and_then(toml::Value::as_str) != Some(crate_name) {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "crate.name",
            message: format!("expected declaration for crate `{crate_name}`"),
        });
    }
    let Some(version) = crate_table.get("version").and_then(toml::Value::as_str) else {
        return DeclarationLookup::Invalid(DeclarationError {
            path: path.to_path_buf(),
            key: "crate.version",
            message: "missing declaration crate version".to_owned(),
        });
    };
    if let Some(expected_version) = expected_version {
        if version != expected_version {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "crate.version",
                message: format!("expected version `{expected_version}`, found `{version}`"),
            });
        }
    }
    if let Some(declared_hash) = crate_table
        .get("summary_hash")
        .or_else(|| crate_table.get("source_hash"))
        .and_then(toml::Value::as_str)
    {
        let observed_hash = stable_hash(&source_without_declared_hash(&source));
        if declared_hash != observed_hash {
            return DeclarationLookup::Invalid(DeclarationError {
                path: path.to_path_buf(),
                key: "crate.summary_hash",
                message: format!(
                    "expected summary_hash `{declared_hash}`, found `{observed_hash}`"
                ),
            });
        }
    }

    DeclarationLookup::Valid(DeclarationFacts {
        path: path.to_path_buf(),
        version: version.to_owned(),
        schema_version,
        hash: stable_hash(&source),
        types: declaration_type_paths(&parsed),
        functions: declaration_function_paths(&parsed),
        function_facts: declaration_function_facts(&parsed),
        effects: declaration_effects(&parsed),
        adapters: declaration_adapter_paths(&parsed),
        activity_facts: declaration_activity_facts(&parsed),
    })
}

pub(super) fn declaration_covers_call(facts: &DeclarationFacts, call_path: Option<&str>) -> bool {
    let Some(call_path) = call_path else {
        return false;
    };
    facts.functions.iter().any(|path| path == call_path)
}

pub(super) fn typed_call_has_replay_critical_effects(
    facts: &DeclarationFacts,
    call_path: Option<&str>,
) -> bool {
    let Some(call_path) = call_path else {
        return true;
    };
    let Some(function) = facts
        .function_facts
        .iter()
        .find(|fact| fact.path == call_path)
    else {
        return true;
    };
    !function.effects.is_empty()
        || function.simulation.as_deref().is_some_and(|simulation| {
            !matches!(simulation, "pure" | "deterministic" | "none" | "typed")
        })
        || function
            .determinism
            .as_deref()
            .is_some_and(|determinism| !matches!(determinism, "deterministic" | "pure"))
}

fn source_without_declared_hash(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("summary_hash =") && !trimmed.starts_with("source_hash =")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn activity_covers_call(
    facts: Option<&DeclarationFacts>,
    call_path: Option<&str>,
) -> bool {
    let Some(facts) = facts else {
        return false;
    };
    let Some(call_path) = call_path else {
        return false;
    };
    facts
        .activity_facts
        .iter()
        .any(|activity| activity.path == call_path && activity.is_complete())
}

impl DeclarationActivityFact {
    fn is_complete(&self) -> bool {
        self.retry
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            && self
                .idempotency
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            && self.result.as_deref() == Some("record")
            && self
                .compensation
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

fn declaration_type_paths(parsed: &toml::Value) -> Vec<String> {
    table_array_paths(parsed, "type")
}

fn declaration_function_paths(parsed: &toml::Value) -> Vec<String> {
    table_array_paths(parsed, "function")
}

fn declaration_function_facts(parsed: &toml::Value) -> Vec<DeclarationFunctionFact> {
    parsed
        .get("function")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(DeclarationFunctionFact {
                path: entry.get("path").and_then(toml::Value::as_str)?.to_owned(),
                effects: entry
                    .get("effects")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                simulation: entry
                    .get("simulation")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                determinism: entry
                    .get("determinism")
                    .or_else(|| entry.get("deterministic"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn declaration_adapter_paths(parsed: &toml::Value) -> Vec<String> {
    table_array_paths(parsed, "adapter")
}

fn declaration_activity_facts(parsed: &toml::Value) -> Vec<DeclarationActivityFact> {
    parsed
        .get("activity")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(DeclarationActivityFact {
                path: entry.get("path").and_then(toml::Value::as_str)?.to_owned(),
                retry: entry
                    .get("retry")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                idempotency: entry
                    .get("idempotency")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                result: entry
                    .get("result")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                compensation: entry
                    .get("compensation")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn table_array_paths(parsed: &toml::Value, key: &str) -> Vec<String> {
    parsed
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            entry
                .get("path")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn declaration_effects(parsed: &toml::Value) -> Vec<String> {
    let mut effects = Vec::new();
    if let Some(functions) = parsed.get("function").and_then(toml::Value::as_array) {
        for function in functions {
            if let Some(path) = function.get("path").and_then(toml::Value::as_str) {
                effects.push(path.to_owned());
            }
            if let Some(items) = function.get("effects").and_then(toml::Value::as_array) {
                effects.extend(
                    items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
    }
    effects.sort();
    effects.dedup();
    effects
}

pub(super) fn stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
