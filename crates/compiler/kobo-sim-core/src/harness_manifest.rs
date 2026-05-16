#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HarnessManifest {
    pub source_hash: String,
    pub generated_rust_hash: String,
    pub harness_dir: String,
    pub harness_rs_path: String,
    pub command: Vec<String>,
    pub exit_code: i32,
    pub stdout_hash: String,
    pub stderr_hash: String,
    pub event_count: usize,
}
