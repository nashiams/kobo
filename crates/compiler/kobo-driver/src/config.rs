use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use kobo_ir::{
    ErrorPolicy, GuaranteeDimension, GuaranteeLevel, GuaranteePolicy, GuaranteeProfile, LegacyMode,
};

/// Workspace and crate-local configuration after all layers have been merged.
#[derive(Debug, Clone, PartialEq)]
pub struct KoboConfig {
    pub guarantee_policy: GuaranteePolicy,
    pub hot_borrow_threshold: u64,
    pub solver_cluster_limit: usize,
    pub solver_budget_seconds: f64,
    pub lsp_solver_budget_ms: u64,
    pub small_struct_clone_threshold_bytes: usize,
    pub channel_buffer_size: usize,
    pub output_dir: Option<PathBuf>,
    pub package_name: String,
    pub package_version: String,
    pub dependencies: HashMap<String, toml::Value>,
    pub dev_dependencies: HashMap<String, toml::Value>,
    pub build_dependencies: HashMap<String, toml::Value>,
    pub target_dependencies: Vec<CargoTargetDependencyConfig>,
    pub workspace_dependencies: HashMap<String, toml::Value>,
    pub copy_types: Vec<String>,
    pub mutating_methods: Vec<String>,
    pub ecosystem_policy: EcosystemPolicyConfig,
    pub runtime_profile: RuntimeProfileConfig,
    pub sim: SimConfig,
    pub src_dir: PathBuf,
    pub enable_parse_recovery: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProfileConfig {
    pub service_buffer: usize,
    pub service_backpressure: String,
    pub scheduler: String,
    pub record: String,
    pub activity: String,
    pub cancellation: String,
    pub scenario_event_budget: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimConfig {
    pub default_profile: String,
    pub show_backend_choices: bool,
    pub profiles: HashMap<String, SimProfileConfig>,
    pub backends: HashMap<String, SimBackendConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimProfileConfig {
    pub schedule_budget: Option<u64>,
    pub seed_count: Option<u64>,
    pub shrink: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimBackendConfig {
    pub enabled: bool,
    pub scheduler: Option<String>,
    pub replay_token: Option<String>,
    pub max_branches: Option<u64>,
    pub checkpoint_replay: Option<bool>,
}

impl SimConfig {
    pub fn schedule_budget_for(&self, profile: &str) -> Option<u64> {
        self.profiles
            .get(profile)
            .and_then(|config| config.schedule_budget)
    }

    pub fn scheduler_for_backend(&self, backend: &str) -> Option<&str> {
        self.backends
            .get(backend)
            .filter(|config| config.enabled)
            .and_then(|config| config.scheduler.as_deref())
    }
}

impl RuntimeProfileConfig {
    pub fn to_codegen_options(&self) -> kobo_codegen::RuntimeProfileOptions {
        kobo_codegen::RuntimeProfileOptions {
            service_buffer: self.service_buffer,
            service_backpressure: self.service_backpressure.clone(),
            scheduler: self.scheduler.clone(),
            record: self.record.clone(),
            activity: self.activity.clone(),
            cancellation: self.cancellation.clone(),
            scenario_event_budget: self.scenario_event_budget,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcosystemPolicyConfig {
    pub default: kobo_ir::ScenarioBoundaryPolicy,
    pub default_is_configured: bool,
    pub replay_unknown: kobo_ir::ScenarioBoundaryPolicy,
    pub crates: Vec<EcosystemCratePolicy>,
    pub types: Vec<EcosystemTypesPolicy>,
    pub adapters: Vec<EcosystemAdapterPolicy>,
    pub summaries: Vec<EcosystemSummaryPolicy>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CargoTargetDependencyConfig {
    pub target: String,
    pub dependencies: HashMap<String, toml::Value>,
    pub dev_dependencies: HashMap<String, toml::Value>,
    pub build_dependencies: HashMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcosystemCratePolicy {
    pub name: String,
    pub policy: kobo_ir::ScenarioBoundaryPolicy,
    pub reason: Option<String>,
    pub retry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcosystemTypesPolicy {
    pub crate_name: String,
    pub package: String,
    pub version: Option<String>,
    pub source: Option<String>,
    pub registry: Option<String>,
    pub checksum: Option<String>,
    pub compatible_crate: Option<String>,
    pub metadata_path: Option<PathBuf>,
    pub trust_policy: Option<String>,
    pub signed_by: Option<String>,
    pub validated: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcosystemAdapterPolicy {
    pub crate_name: String,
    pub package: String,
    pub version: Option<String>,
    pub confidence: Option<String>,
    pub source: Option<String>,
    pub registry: Option<String>,
    pub checksum: Option<String>,
    pub compatible_crate: Option<String>,
    pub metadata_path: Option<PathBuf>,
    pub trust_policy: Option<String>,
    pub signed_by: Option<String>,
    pub validated: bool,
    pub adapter_runtime: Option<String>,
    pub capture: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcosystemSummaryPolicy {
    pub crate_name: String,
    pub path: PathBuf,
    pub hash: String,
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
    #[serde(default)]
    package: RawPackageSection,
    #[serde(default)]
    dependencies: HashMap<String, toml::Value>,
    #[serde(default)]
    copy_types: RawCopyTypesSection,
    #[serde(default)]
    mutating_methods: RawMutatingMethodsSection,
    #[serde(default)]
    guarantees: RawGuaranteesSection,
    #[serde(default)]
    ecosystem: RawEcosystemSection,
    #[serde(default)]
    runtime: RawRuntimeSection,
    #[serde(default)]
    sim: RawSimSection,
    #[serde(default)]
    profiles: HashMap<String, RawProfileSection>,
    mode: Option<LegacyMode>,
    profile: Option<GuaranteeProfile>,
    hot_borrow_threshold: Option<u64>,
    solver_cluster_limit: Option<usize>,
    solver_budget_seconds: Option<f64>,
    lsp_solver_budget_ms: Option<u64>,
    small_struct_clone_threshold_bytes: Option<usize>,
    channel_buffer_size: Option<usize>,
    output_dir: Option<PathBuf>,
    enable_parse_recovery: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKoboSection {
    mode: Option<LegacyMode>,
    profile: Option<GuaranteeProfile>,
    output_dir: Option<PathBuf>,
    enable_parse_recovery: Option<bool>,
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

#[derive(Debug, Default, Deserialize)]
struct RawPackageSection {
    name: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCopyTypesSection {
    #[serde(default)]
    external: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawMutatingMethodsSection {
    #[serde(default)]
    methods: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawProfileSection {
    #[serde(default)]
    guarantees: RawGuaranteesSection,
}

#[derive(Debug, Default, Deserialize)]
struct RawGuaranteesSection {
    ownership: Option<GuaranteeLevel>,
    liveness: Option<GuaranteeLevel>,
    replay: Option<GuaranteeLevel>,
    boundaries: Option<GuaranteeLevel>,
    errors: Option<ErrorPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEcosystemSection {
    default: Option<String>,
    replay_unknown: Option<String>,
    #[serde(default, rename = "crate")]
    crates: Vec<RawEcosystemCrateSection>,
    #[serde(default)]
    types: Vec<RawEcosystemTypesSection>,
    #[serde(default)]
    adapter: Vec<RawEcosystemAdapterSection>,
    #[serde(default)]
    summary: Vec<RawEcosystemSummarySection>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRuntimeSection {
    #[serde(default)]
    profile: RawRuntimeProfileSection,
}

#[derive(Debug, Default, Deserialize)]
struct RawSimSection {
    default_profile: Option<String>,
    show_backend_choices: Option<bool>,
    #[serde(default)]
    profile: HashMap<String, RawSimProfileSection>,
    #[serde(default)]
    backend: HashMap<String, RawSimBackendSection>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRuntimeProfileSection {
    service_buffer: Option<usize>,
    service_backpressure: Option<String>,
    scheduler: Option<String>,
    record: Option<String>,
    activity: Option<String>,
    cancellation: Option<String>,
    scenario_event_budget: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSimProfileSection {
    schedule_budget: Option<u64>,
    seed_count: Option<u64>,
    shrink: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSimBackendSection {
    enabled: Option<bool>,
    scheduler: Option<String>,
    replay_token: Option<String>,
    max_branches: Option<u64>,
    checkpoint_replay: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEcosystemTypesSection {
    #[serde(rename = "crate")]
    crate_name: String,
    package: String,
    version: Option<String>,
    source: Option<String>,
    registry: Option<String>,
    checksum: Option<String>,
    compatible_crate: Option<String>,
    metadata_path: Option<PathBuf>,
    trust_policy: Option<String>,
    signed_by: Option<String>,
    validated: Option<bool>,
    reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEcosystemCrateSection {
    name: String,
    policy: Option<String>,
    reason: Option<String>,
    retry: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEcosystemAdapterSection {
    #[serde(rename = "crate")]
    crate_name: String,
    package: String,
    version: Option<String>,
    confidence: Option<String>,
    source: Option<String>,
    registry: Option<String>,
    checksum: Option<String>,
    compatible_crate: Option<String>,
    metadata_path: Option<PathBuf>,
    trust_policy: Option<String>,
    signed_by: Option<String>,
    validated: Option<bool>,
    adapter_runtime: Option<String>,
    capture: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEcosystemSummarySection {
    #[serde(rename = "crate")]
    crate_name: String,
    path: PathBuf,
    hash: String,
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
        source: Box<toml::de::Error>,
    },
    #[error("failed to parse TOML: {0}")]
    ParseError(String),

    #[error("K0120 ecosystem policy parse error at {key}: unknown boundary policy `{value}`")]
    InvalidEcosystemPolicy { key: String, value: String },

    #[error("K0121 simulation config parse error at {key}: {message}")]
    InvalidSimConfig { key: String, message: String },
}

impl Default for KoboConfig {
    fn default() -> Self {
        Self {
            guarantee_policy: GuaranteePolicy::default(),
            hot_borrow_threshold: 10_000,
            solver_cluster_limit: 2048,
            solver_budget_seconds: 5.0,
            lsp_solver_budget_ms: 200,
            small_struct_clone_threshold_bytes: 128,
            channel_buffer_size: 100,
            output_dir: None,
            package_name: String::new(),
            package_version: String::new(),
            dependencies: HashMap::new(),
            dev_dependencies: HashMap::new(),
            build_dependencies: HashMap::new(),
            target_dependencies: Vec::new(),
            workspace_dependencies: HashMap::new(),
            copy_types: Vec::new(),
            mutating_methods: Vec::new(),
            ecosystem_policy: EcosystemPolicyConfig::default(),
            runtime_profile: RuntimeProfileConfig::default(),
            sim: SimConfig::default(),
            src_dir: PathBuf::from("src"),
            enable_parse_recovery: false,
        }
    }
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            default_profile: "quick".to_owned(),
            show_backend_choices: false,
            profiles: HashMap::new(),
            backends: HashMap::new(),
        }
    }
}

impl Default for SimBackendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scheduler: None,
            replay_token: None,
            max_branches: None,
            checkpoint_replay: None,
        }
    }
}

impl Default for RuntimeProfileConfig {
    fn default() -> Self {
        Self {
            service_buffer: 64,
            service_backpressure: "block-on-full".to_owned(),
            scheduler: "small-random".to_owned(),
            record: "recorded-boundary-io".to_owned(),
            activity: "retry-idempotency".to_owned(),
            cancellation: "scheduler-history".to_owned(),
            scenario_event_budget: 64,
        }
    }
}

impl Default for EcosystemPolicyConfig {
    fn default() -> Self {
        Self {
            default: kobo_ir::ScenarioBoundaryPolicy::Opaque,
            default_is_configured: false,
            replay_unknown: kobo_ir::ScenarioBoundaryPolicy::Debt,
            crates: Vec::new(),
            types: Vec::new(),
            adapters: Vec::new(),
            summaries: Vec::new(),
        }
    }
}

impl EcosystemPolicyConfig {
    pub fn crate_policy(&self, crate_name: &str) -> Option<&EcosystemCratePolicy> {
        self.crates.iter().find(|policy| policy.name == crate_name)
    }

    pub fn adapter_for(&self, crate_name: &str) -> Option<&EcosystemAdapterPolicy> {
        self.adapters
            .iter()
            .find(|adapter| adapter.crate_name == crate_name)
    }

    pub fn types_for(&self, crate_name: &str) -> Option<&EcosystemTypesPolicy> {
        self.types
            .iter()
            .find(|types| types.crate_name == crate_name)
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
    apply_cargo_manifest_layer(&mut config, workspace_root)?;

    if crate_dir != workspace_root {
        apply_config_layer(&mut config, crate_dir)?;
        apply_cargo_manifest_layer(&mut config, crate_dir)?;
    }

    Ok(config)
}

fn apply_config_layer(config: &mut KoboConfig, config_dir: &Path) -> Result<(), ConfigError> {
    let config_path = config_dir.join("Kobo.toml");
    let Some(raw_config) = read_optional_config(&config_path)? else {
        return Ok(());
    };

    raw_config.merge_into(config, config_dir)?;
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
            source: Box::new(source),
        })
}

fn apply_cargo_manifest_layer(
    config: &mut KoboConfig,
    manifest_dir: &Path,
) -> Result<(), ConfigError> {
    let manifest_path = manifest_dir.join("Cargo.toml");
    if !manifest_path.exists() {
        return Ok(());
    }

    let source = fs::read_to_string(&manifest_path).map_err(|source| ConfigError::Io {
        path: manifest_path.clone(),
        source,
    })?;
    let parsed = source
        .parse::<toml::Value>()
        .map_err(|source| ConfigError::Parse {
            path: manifest_path,
            source: Box::new(source),
        })?;

    if let Some(package) = parsed.get("package").and_then(toml::Value::as_table) {
        if let Some(name) = package.get("name").and_then(toml::Value::as_str) {
            config.package_name = name.to_owned();
        }
        if let Some(version) = package.get("version").and_then(toml::Value::as_str) {
            config.package_version = version.to_owned();
        }
    }

    if let Some(workspace_dependencies) = parsed
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        config.workspace_dependencies = workspace_dependencies
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
    }

    config.dependencies = dependency_section(&parsed, "dependencies");
    config.dev_dependencies = dependency_section(&parsed, "dev-dependencies");
    config.build_dependencies = dependency_section(&parsed, "build-dependencies");
    config.target_dependencies = target_dependency_sections(&parsed);
    Ok(())
}

fn dependency_section(parsed: &toml::Value, section: &str) -> HashMap<String, toml::Value> {
    parsed
        .get(section)
        .and_then(toml::Value::as_table)
        .map(|dependencies| {
            dependencies
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn target_dependency_sections(parsed: &toml::Value) -> Vec<CargoTargetDependencyConfig> {
    let mut sections = Vec::new();
    let Some(targets) = parsed.get("target").and_then(toml::Value::as_table) else {
        return sections;
    };
    for (target, value) in targets {
        let dependencies = dependency_section(value, "dependencies");
        let dev_dependencies = dependency_section(value, "dev-dependencies");
        let build_dependencies = dependency_section(value, "build-dependencies");
        if !dependencies.is_empty()
            || !dev_dependencies.is_empty()
            || !build_dependencies.is_empty()
        {
            sections.push(CargoTargetDependencyConfig {
                target: target.clone(),
                dependencies,
                dev_dependencies,
                build_dependencies,
            });
        }
    }
    sections.sort_by(|left, right| left.target.cmp(&right.target));
    sections
}

impl RawKoboConfig {
    fn merge_into(self, config: &mut KoboConfig, config_dir: &Path) -> Result<(), ConfigError> {
        if let Some(profile) = self.profile.or(self.kobo.profile).or_else(|| {
            self.mode
                .or(self.kobo.mode)
                .map(LegacyMode::guarantee_profile)
        }) {
            config.guarantee_policy = GuaranteePolicy::for_profile(profile);
        }
        self.guarantees.apply_to(&mut config.guarantee_policy);
        if let Some(profile_config) = self
            .profiles
            .get(config.guarantee_policy.profile().as_str())
        {
            profile_config
                .guarantees
                .apply_to(&mut config.guarantee_policy);
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

        if let Some(channel_buffer_size) = self.channel_buffer_size {
            config.channel_buffer_size = channel_buffer_size;
        }

        if let Some(enable_parse_recovery) = self
            .enable_parse_recovery
            .or(self.kobo.enable_parse_recovery)
        {
            config.enable_parse_recovery = enable_parse_recovery;
        }

        if let Some(output_dir) = self
            .output_dir
            .or(self.kobo.output_dir)
            .or(self.output.output_dir)
        {
            config.output_dir = Some(resolve_output_dir(config_dir, output_dir));
        }

        if let Some(name) = self.package.name {
            config.package_name = name;
        }
        if let Some(version) = self.package.version {
            config.package_version = version;
        }
        if !self.dependencies.is_empty() {
            config.dependencies = self.dependencies;
        }
        if !self.copy_types.external.is_empty() {
            config.copy_types = self.copy_types.external;
        }
        if !self.mutating_methods.methods.is_empty() {
            config.mutating_methods = self.mutating_methods.methods;
        }
        self.ecosystem
            .apply_to(&mut config.ecosystem_policy, config_dir)?;
        self.runtime.profile.apply_to(&mut config.runtime_profile);
        self.sim.apply_to(&mut config.sim)?;
        Ok(())
    }
}

impl RawSimSection {
    fn apply_to(self, sim: &mut SimConfig) -> Result<(), ConfigError> {
        if let Some(default_profile) = self.default_profile {
            validate_sim_profile_name("[sim].default_profile", &default_profile)?;
            sim.default_profile = default_profile;
        }
        if let Some(show_backend_choices) = self.show_backend_choices {
            sim.show_backend_choices = show_backend_choices;
        }
        for (name, profile) in self.profile {
            validate_sim_profile_name("[sim.profile]", &name)?;
            validate_sim_shrink("[sim.profile].shrink", profile.shrink.as_deref())?;
            sim.profiles.insert(
                name,
                SimProfileConfig {
                    schedule_budget: profile.schedule_budget,
                    seed_count: profile.seed_count,
                    shrink: profile.shrink,
                },
            );
        }
        for (name, backend) in self.backend {
            validate_sim_backend_name("[sim.backend]", &name)?;
            validate_sim_backend_scheduler(&name, backend.scheduler.as_deref())?;
            validate_sim_replay_token(&name, backend.replay_token.as_deref())?;
            sim.backends.insert(
                name,
                SimBackendConfig {
                    enabled: backend.enabled.unwrap_or(true),
                    scheduler: backend.scheduler,
                    replay_token: backend.replay_token,
                    max_branches: backend.max_branches,
                    checkpoint_replay: backend.checkpoint_replay,
                },
            );
        }
        Ok(())
    }
}

fn validate_sim_profile_name(key: &str, profile: &str) -> Result<(), ConfigError> {
    if matches!(profile, "quick" | "deep" | "replay" | "exhaustive") {
        return Ok(());
    }
    Err(ConfigError::InvalidSimConfig {
        key: key.to_owned(),
        message: format!(
            "unknown simulation profile `{profile}`; expected quick, deep, replay, or exhaustive"
        ),
    })
}

fn validate_sim_backend_name(key: &str, backend: &str) -> Result<(), ConfigError> {
    if matches!(
        backend,
        "loom" | "shuttle" | "turmoil" | "madsim" | "proptest" | "failpoints"
    ) {
        return Ok(());
    }
    Err(ConfigError::InvalidSimConfig {
        key: key.to_owned(),
        message: format!(
            "unknown simulation backend `{backend}`; expected loom, shuttle, turmoil, madsim, proptest, or failpoints"
        ),
    })
}

fn validate_sim_backend_scheduler(
    backend: &str,
    scheduler: Option<&str>,
) -> Result<(), ConfigError> {
    let Some(scheduler) = scheduler else {
        return Ok(());
    };
    let supported = match backend {
        "shuttle" => matches!(scheduler, "pct" | "pct-random-bounded" | "small-random"),
        "loom" => matches!(scheduler, "exhaustive" | "small-random"),
        "turmoil" | "madsim" => matches!(scheduler, "deterministic" | "small-random"),
        "proptest" | "failpoints" => scheduler == "small-random",
        _ => false,
    };
    if supported {
        return Ok(());
    }
    Err(ConfigError::InvalidSimConfig {
        key: format!("[sim.backend.{backend}].scheduler"),
        message: format!("unsupported scheduler `{scheduler}` for backend `{backend}`"),
    })
}

fn validate_sim_replay_token(backend: &str, replay_token: Option<&str>) -> Result<(), ConfigError> {
    let Some(replay_token) = replay_token else {
        return Ok(());
    };
    if matches!(replay_token, "record" | "metadata" | "none") {
        return Ok(());
    }
    Err(ConfigError::InvalidSimConfig {
        key: format!("[sim.backend.{backend}].replay_token"),
        message: format!(
            "unsupported replay token `{replay_token}`; expected record, metadata, or none"
        ),
    })
}

fn validate_sim_shrink(key: &str, shrink: Option<&str>) -> Result<(), ConfigError> {
    let Some(shrink) = shrink else {
        return Ok(());
    };
    if matches!(shrink, "off" | "best-effort") {
        return Ok(());
    }
    Err(ConfigError::InvalidSimConfig {
        key: key.to_owned(),
        message: format!("unsupported shrink mode `{shrink}`; expected off or best-effort"),
    })
}

impl RawRuntimeProfileSection {
    fn apply_to(self, profile: &mut RuntimeProfileConfig) {
        if let Some(service_buffer) = self.service_buffer {
            profile.service_buffer = service_buffer;
        }
        if let Some(service_backpressure) = self.service_backpressure {
            profile.service_backpressure = service_backpressure;
        }
        if let Some(scheduler) = self.scheduler {
            profile.scheduler = scheduler;
        }
        if let Some(record) = self.record {
            profile.record = record;
        }
        if let Some(activity) = self.activity {
            profile.activity = activity;
        }
        if let Some(cancellation) = self.cancellation {
            profile.cancellation = cancellation;
        }
        if let Some(scenario_event_budget) = self.scenario_event_budget {
            profile.scenario_event_budget = scenario_event_budget;
        }
    }
}

impl RawEcosystemSection {
    fn apply_to(
        self,
        policy: &mut EcosystemPolicyConfig,
        config_dir: &Path,
    ) -> Result<(), ConfigError> {
        if let Some(default) = self.default.as_deref() {
            let default = parse_boundary_policy("ecosystem.default", default)?;
            policy.default = default;
            policy.default_is_configured = true;
        }
        if let Some(replay_unknown) = self.replay_unknown.as_deref() {
            let replay_unknown = parse_boundary_policy("ecosystem.replay_unknown", replay_unknown)?;
            policy.replay_unknown = replay_unknown;
        }
        for crate_policy in self.crates {
            policy
                .crates
                .retain(|existing| existing.name != crate_policy.name);
            policy.crates.push(EcosystemCratePolicy {
                name: crate_policy.name,
                policy: match crate_policy.policy.as_deref() {
                    Some(value) => parse_boundary_policy("ecosystem.crate.policy", value)?,
                    None => policy.default.clone(),
                },
                reason: crate_policy.reason,
                retry: crate_policy.retry,
            });
        }
        for types in self.types {
            policy
                .types
                .retain(|existing| existing.crate_name != types.crate_name);
            policy.types.push(EcosystemTypesPolicy {
                crate_name: types.crate_name,
                package: types.package,
                version: types.version,
                source: types.source,
                registry: types.registry,
                checksum: types.checksum,
                compatible_crate: types.compatible_crate,
                metadata_path: types
                    .metadata_path
                    .map(|path| resolve_output_dir(config_dir, path)),
                trust_policy: types.trust_policy,
                signed_by: types.signed_by,
                validated: types.validated.unwrap_or(false),
                reason: types.reason,
            });
        }
        for adapter in self.adapter {
            policy
                .adapters
                .retain(|existing| existing.crate_name != adapter.crate_name);
            policy.adapters.push(EcosystemAdapterPolicy {
                crate_name: adapter.crate_name,
                package: adapter.package,
                version: adapter.version,
                confidence: adapter.confidence,
                source: adapter.source,
                registry: adapter.registry,
                checksum: adapter.checksum,
                compatible_crate: adapter.compatible_crate,
                metadata_path: adapter
                    .metadata_path
                    .map(|path| resolve_output_dir(config_dir, path)),
                trust_policy: adapter.trust_policy,
                signed_by: adapter.signed_by,
                validated: adapter.validated.unwrap_or(false),
                adapter_runtime: adapter.adapter_runtime,
                capture: adapter.capture,
                reason: adapter.reason,
            });
        }
        for summary in self.summary {
            policy
                .summaries
                .retain(|existing| existing.crate_name != summary.crate_name);
            policy.summaries.push(EcosystemSummaryPolicy {
                crate_name: summary.crate_name,
                path: resolve_output_dir(config_dir, summary.path),
                hash: summary.hash,
            });
        }
        Ok(())
    }
}

fn parse_boundary_policy(
    key: &str,
    value: &str,
) -> Result<kobo_ir::ScenarioBoundaryPolicy, ConfigError> {
    let policy = kobo_ir::ScenarioBoundaryPolicy::from_str(value);
    if matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Unselected) && value != "unselected" {
        return Err(ConfigError::InvalidEcosystemPolicy {
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(policy)
}

impl RawGuaranteesSection {
    fn apply_to(&self, policy: &mut GuaranteePolicy) {
        if let Some(level) = self.ownership {
            policy
                .guarantees_mut()
                .set_level(GuaranteeDimension::Ownership, level);
        }
        if let Some(level) = self.liveness {
            policy
                .guarantees_mut()
                .set_level(GuaranteeDimension::Liveness, level);
        }
        if let Some(level) = self.replay {
            policy
                .guarantees_mut()
                .set_level(GuaranteeDimension::Replay, level);
        }
        if let Some(level) = self.boundaries {
            policy
                .guarantees_mut()
                .set_level(GuaranteeDimension::Boundaries, level);
        }
        if let Some(errors) = self.errors {
            policy.guarantees_mut().set_errors(errors);
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

/// Parse a `Kobo.toml` from its raw TOML string, returning a fully-merged `KoboConfig`.
///
/// Does **not** resolve output_dir against a filesystem path — the caller should handle that.
pub fn parse_kobo_config(toml_str: &str) -> Result<KoboConfig, ConfigError> {
    let raw: RawKoboConfig =
        toml::from_str(toml_str).map_err(|e| ConfigError::ParseError(e.to_string()))?;
    let mut config = KoboConfig::default();
    // Use a dummy config_dir since we can't resolve paths from a raw string.
    let dummy_dir = Path::new(".");
    raw.merge_into(&mut config, dummy_dir)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{load_config, load_config_for, GuaranteeProfile};

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

        assert_eq!(config.guarantee_policy.profile(), GuaranteeProfile::Dev);
        assert_eq!(config.hot_borrow_threshold, 10_000);
        assert_eq!(config.solver_cluster_limit, 2048);
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

        assert_eq!(config.guarantee_policy.profile(), GuaranteeProfile::Release);
        assert_eq!(config.hot_borrow_threshold, 1200);
        assert_eq!(config.solver_cluster_limit, 400);
        assert_eq!(config.solver_budget_seconds, 7.5);
        assert_eq!(config.lsp_solver_budget_ms, 250);
        assert_eq!(config.small_struct_clone_threshold_bytes, 192);
        assert_eq!(config.output_dir, Some(crate_dir.join("generated")));
    }

    // ── Phase 1 / Step 1.1: parse_kobo_config tests ──────────────────

    use super::parse_kobo_config;

    #[test]
    fn parse_config_loads_package_name() {
        let toml = r#"
[package]
name = "my-app"
version = "0.2.0"
"#;
        let config = parse_kobo_config(toml).unwrap();
        assert_eq!(config.package_name, "my-app");
        assert_eq!(config.package_version, "0.2.0");
    }

    #[test]
    fn parse_config_loads_dependencies() {
        let toml = r#"
[dependencies]
serde = "1"
tokio = { version = "1", features = ["full"] }
"#;
        let config = parse_kobo_config(toml).unwrap();
        assert_eq!(config.dependencies.len(), 2);
        assert!(config.dependencies.contains_key("serde"));
        assert!(config.dependencies.contains_key("tokio"));
    }

    #[test]
    fn parse_config_loads_copy_types() {
        let toml = r#"
[copy_types]
external = ["MyPoint", "Color"]
"#;
        let config = parse_kobo_config(toml).unwrap();
        assert_eq!(config.copy_types, vec!["MyPoint", "Color"]);
    }

    #[test]
    fn parse_config_loads_mutating_methods() {
        let toml = r#"
[mutating_methods]
methods = ["push", "insert", "remove"]
"#;
        let config = parse_kobo_config(toml).unwrap();
        assert_eq!(config.mutating_methods, vec!["push", "insert", "remove"]);
    }

    #[test]
    fn parse_config_loads_sim_profiles_and_backends() {
        let toml = r#"
[sim]
default_profile = "quick"
show_backend_choices = true

[sim.profile.quick]
schedule_budget = 7
seed_count = 3
shrink = "off"

[sim.backend.shuttle]
enabled = true
scheduler = "pct"
replay_token = "record"
max_branches = 11
checkpoint_replay = true
"#;
        let config = parse_kobo_config(toml).unwrap();
        let profile = config.sim.profiles.get("quick").unwrap();
        let backend = config.sim.backends.get("shuttle").unwrap();

        assert_eq!(config.sim.default_profile, "quick");
        assert!(config.sim.show_backend_choices);
        assert_eq!(profile.schedule_budget, Some(7));
        assert_eq!(profile.seed_count, Some(3));
        assert_eq!(profile.shrink.as_deref(), Some("off"));
        assert!(backend.enabled);
        assert_eq!(backend.scheduler.as_deref(), Some("pct"));
        assert_eq!(backend.replay_token.as_deref(), Some("record"));
        assert_eq!(backend.max_branches, Some(11));
        assert_eq!(backend.checkpoint_replay, Some(true));
    }

    #[test]
    fn parse_config_rejects_unstable_sim_profile_and_backend_keys() {
        let profile_error = parse_kobo_config(
            r#"
[sim.profile.shuttle-ish]
schedule_budget = 7
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            profile_error.contains("unknown simulation profile"),
            "{profile_error}"
        );

        let backend_error = parse_kobo_config(
            r#"
[sim.backend.fake]
scheduler = "pct"
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            backend_error.contains("unknown simulation backend"),
            "{backend_error}"
        );
    }

    #[test]
    fn parse_config_defaults_when_empty() {
        let config = parse_kobo_config("").unwrap();
        assert_eq!(config.package_name, "");
        assert!(config.dependencies.is_empty());
        assert!(config.copy_types.is_empty());
        assert!(config.mutating_methods.is_empty());
        assert_eq!(config.src_dir, PathBuf::from("src"));
    }
}
