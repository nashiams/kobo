use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use kobo_ir::KoboMode;

/// Workspace and crate-local configuration after all layers have been merged.
#[derive(Debug, Clone, PartialEq)]
pub struct KoboConfig {
    pub mode: KoboMode,
    pub hot_borrow_threshold: u64,
    pub solver_cluster_limit: usize,
    pub solver_budget_seconds: f64,
    pub lsp_solver_budget_ms: u64,
    pub small_struct_clone_threshold_bytes: usize,
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKoboConfig {
    #[serde(default)]
    kobo: RawKoboSection,
    #[serde(default)]
    diagnostics: RawDiagnosticsSection,
    #[serde(default)]
    solver: RawSolverSection,
    #[serde(default)]
    transform: RawTransformSection,
    #[serde(default)]
    output: RawOutputSection,
    mode: Option<KoboMode>,
    hot_borrow_threshold: Option<u64>,
    solver_cluster_limit: Option<usize>,
    solver_budget_seconds: Option<f64>,
    lsp_solver_budget_ms: Option<u64>,
    small_struct_clone_threshold_bytes: Option<usize>,
    output_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKoboSection {
    mode: Option<KoboMode>,
    output_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDiagnosticsSection {
    hot_borrow_threshold: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSolverSection {
    cluster_limit: Option<usize>,
    budget_seconds: Option<f64>,
    lsp_budget_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTransformSection {
    small_struct_clone_threshold_bytes: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOutputSection {
    output_dir: Option<PathBuf>,
}

/// Errors produced while loading or parsing `Kobo.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read Kobo.toml at {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse Kobo.toml at {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Default for KoboConfig {
    fn default() -> Self {
        Self {
            mode: KoboMode::Script,
            hot_borrow_threshold: 10_000,
            solver_cluster_limit: 256,
            solver_budget_seconds: 5.0,
            lsp_solver_budget_ms: 200,
            small_struct_clone_threshold_bytes: 128,
            output_dir: None,
        }
    }
}

/// Loads the workspace-level configuration from `workspace_root/Kobo.toml`.
pub fn load_config(workspace_root: &Path) -> Result<KoboConfig, ConfigError> {
    load_config_for(workspace_root, workspace_root)
}

/// Loads workspace config, then overlays `<crate_dir>/Kobo.toml` if it exists.
pub fn load_config_for(crate_dir: &Path, workspace_root: &Path) -> Result<KoboConfig, ConfigError> {
    let mut config = KoboConfig::default();

    apply_config_layer(&mut config, workspace_root)?;

    if crate_dir != workspace_root {
        apply_config_layer(&mut config, crate_dir)?;
    }

    Ok(config)
}

fn apply_config_layer(config: &mut KoboConfig, config_dir: &Path) -> Result<(), ConfigError> {
    let config_path = config_dir.join("Kobo.toml");
    let Some(raw_config) = read_optional_config(&config_path)? else {
        return Ok(());
    };

    raw_config.merge_into(config, config_dir);
    Ok(())
}

fn read_optional_config(config_path: &Path) -> Result<Option<RawKoboConfig>, ConfigError> {
    if !config_path.exists() {
        return Ok(None);
    }

    let source = fs::read_to_string(config_path).map_err(|source| ConfigError::Io {
        path: config_path.to_path_buf(),
        source,
    })?;

    toml::from_str(&source)
        .map(Some)
        .map_err(|source| ConfigError::Parse {
            path: config_path.to_path_buf(),
            source,
        })
}

impl RawKoboConfig {
    fn merge_into(self, config: &mut KoboConfig, config_dir: &Path) {
        if let Some(mode) = self.mode.or(self.kobo.mode) {
            config.mode = mode;
        }

        if let Some(hot_borrow_threshold) = self
            .hot_borrow_threshold
            .or(self.diagnostics.hot_borrow_threshold)
        {
            config.hot_borrow_threshold = hot_borrow_threshold;
        }

        if let Some(solver_cluster_limit) = self.solver_cluster_limit.or(self.solver.cluster_limit)
        {
            config.solver_cluster_limit = solver_cluster_limit;
        }

        if let Some(solver_budget_seconds) =
            self.solver_budget_seconds.or(self.solver.budget_seconds)
        {
            config.solver_budget_seconds = solver_budget_seconds;
        }

        if let Some(lsp_solver_budget_ms) = self.lsp_solver_budget_ms.or(self.solver.lsp_budget_ms)
        {
            config.lsp_solver_budget_ms = lsp_solver_budget_ms;
        }

        if let Some(small_struct_clone_threshold_bytes) = self
            .small_struct_clone_threshold_bytes
            .or(self.transform.small_struct_clone_threshold_bytes)
        {
            config.small_struct_clone_threshold_bytes = small_struct_clone_threshold_bytes;
        }

        if let Some(output_dir) = self
            .output_dir
            .or(self.kobo.output_dir)
            .or(self.output.output_dir)
        {
            config.output_dir = Some(resolve_output_dir(config_dir, output_dir));
        }
    }
}

fn resolve_output_dir(config_dir: &Path, output_dir: PathBuf) -> PathBuf {
    if output_dir.is_absolute() {
        output_dir
    } else {
        config_dir.join(output_dir)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{load_config, load_config_for, KoboMode};

    static TEST_DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let suffix = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "kobo-driver-config-{name}-{}-{suffix}",
                std::process::id()
            ));

            if path.exists() {
                let _ = fs::remove_dir_all(&path);
            }

            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn load_config_uses_defaults_when_file_is_absent() {
        let workspace_root = TestDir::new("defaults");
        let config = load_config(workspace_root.path()).unwrap();

        assert_eq!(config.mode, KoboMode::Script);
        assert_eq!(config.hot_borrow_threshold, 10_000);
        assert_eq!(config.solver_cluster_limit, 256);
        assert_eq!(config.small_struct_clone_threshold_bytes, 128);
        assert_eq!(config.output_dir, None);
    }

    #[test]
    fn load_config_for_merges_workspace_and_crate_layers() {
        let workspace_root = TestDir::new("merge");
        let crate_dir = workspace_root.path().join("crates").join("demo");
        fs::create_dir_all(&crate_dir).unwrap();

        fs::write(
            workspace_root.path().join("Kobo.toml"),
            r#"
[kobo]
mode = "checked"

[diagnostics]
hot_borrow_threshold = 1200

[solver]
cluster_limit = 400
budget_seconds = 7.5
lsp_budget_ms = 250

[transform]
small_struct_clone_threshold_bytes = 192
"#,
        )
        .unwrap();

        fs::write(
            crate_dir.join("Kobo.toml"),
            r#"
[kobo]
mode = "strict"

[output]
output_dir = "generated"
"#,
        )
        .unwrap();

        let config = load_config_for(&crate_dir, workspace_root.path()).unwrap();

        assert_eq!(config.mode, KoboMode::Strict);
        assert_eq!(config.hot_borrow_threshold, 1200);
        assert_eq!(config.solver_cluster_limit, 400);
        assert_eq!(config.solver_budget_seconds, 7.5);
        assert_eq!(config.lsp_solver_budget_ms, 250);
        assert_eq!(config.small_struct_clone_threshold_bytes, 192);
        assert_eq!(config.output_dir, Some(crate_dir.join("generated")));
    }
}
