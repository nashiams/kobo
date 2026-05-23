#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceHookEvent {
    pub phase: String,
    pub service: String,
    pub method: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HarnessManifest {
    pub source_hash: String,
    pub generated_rust_hash: String,
    pub execution_scope: String,
    pub full_ecosystem_exploration: bool,
    pub facades: Vec<String>,
    pub harness_dir: String,
    pub harness_rs_path: String,
    pub command: Vec<String>,
    pub exit_code: i32,
    pub stdout_hash: String,
    pub stderr_hash: String,
    pub event_count: usize,
    pub service_hook_events: Vec<ServiceHookEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_path: Option<String>,
}
