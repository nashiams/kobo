use crate::core::ScenarioEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendCapability {
    pub name: &'static str,
    pub display_name: &'static str,
    pub executes_now: bool,
    pub integration_level: &'static str,
    pub scenario_execution: &'static str,
    pub ecosystem_scope: &'static str,
    pub full_ecosystem_exploration: bool,
    pub registered_boundary_exploration: bool,
    pub coverage_scope: &'static str,
    pub role: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataBackendCapability {
    pub name: &'static str,
    pub display_name: &'static str,
    pub backend_fit: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendExecution {
    pub engine: &'static str,
    pub integration_level: &'static str,
    pub scenario_execution: &'static str,
    pub token_material: String,
}

pub fn capabilities() -> &'static [BackendCapability] {
    &[
        BackendCapability {
            name: "generated-rust-process",
            display_name: "generated-rust-process",
            executes_now: true,
            integration_level: "generated-user-rust",
            scenario_execution: "kobo-managed-generated-rust",
            ecosystem_scope: "generated-user-rust",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: true,
            coverage_scope: "registered-boundaries-generated-rust",
            role: "semantic/harness agreement and replay token evidence",
        },
        BackendCapability {
            name: "proptest",
            display_name: "proptest",
            executes_now: true,
            integration_level: "input-generation",
            scenario_execution: "stateful-generated-inputs",
            ecosystem_scope: "generated-inputs",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: false,
            coverage_scope: "generated-inputs-only",
            role: "stateful input generation for --fuzz",
        },
        BackendCapability {
            name: "loom",
            display_name: "Loom",
            executes_now: true,
            integration_level: "generated-user-rust",
            scenario_execution: "generated-user-rust-loom",
            ecosystem_scope: "real-loom-generated-user-rust",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: true,
            coverage_scope: "registered-boundaries-loom-generated-rust",
            role: "sync profile wraps generated user Rust in loom::model",
        },
        BackendCapability {
            name: "shuttle",
            display_name: "Shuttle",
            executes_now: false,
            integration_level: "metadata-only",
            scenario_execution: "unsupported-native-adapter",
            ecosystem_scope: "backend-native-unsupported",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: false,
            coverage_scope: "unsupported-native-adapter",
            role: "reserved backend pin; native Shuttle execution is rejected until an adapter is linked",
        },
        BackendCapability {
            name: "turmoil",
            display_name: "Turmoil",
            executes_now: false,
            integration_level: "metadata-only",
            scenario_execution: "unsupported-native-adapter",
            ecosystem_scope: "backend-native-unsupported",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: false,
            coverage_scope: "unsupported-native-adapter",
            role: "reserved backend pin; native Turmoil execution is rejected until an adapter is linked",
        },
        BackendCapability {
            name: "madsim",
            display_name: "Madsim",
            executes_now: false,
            integration_level: "metadata-only",
            scenario_execution: "unsupported-native-adapter",
            ecosystem_scope: "backend-native-unsupported",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: false,
            coverage_scope: "unsupported-native-adapter",
            role: "reserved backend pin; native Madsim execution is rejected until an adapter is linked",
        },
        BackendCapability {
            name: "storage-filesystem",
            display_name: "storage-filesystem",
            executes_now: true,
            integration_level: "generated-user-rust",
            scenario_execution: "generated-user-rust-filesystem",
            ecosystem_scope: "generated-user-rust-os-facade",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: true,
            coverage_scope: "registered-boundaries-storage-facade",
            role: "storage facade executes generated Rust filesystem operations",
        },
        BackendCapability {
            name: "network-loopback",
            display_name: "network-loopback",
            executes_now: true,
            integration_level: "generated-user-rust",
            scenario_execution: "generated-user-rust-loopback-network",
            ecosystem_scope: "generated-user-rust-os-facade",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: true,
            coverage_scope: "registered-boundaries-network-facade",
            role: "network facade executes generated Rust loopback socket operations",
        },
        BackendCapability {
            name: "failpoints",
            display_name: "failpoints",
            executes_now: true,
            integration_level: "generated-user-rust-adapter",
            scenario_execution: "kobo-managed-failure-hooks",
            ecosystem_scope: "generated-user-rust-adapter",
            full_ecosystem_exploration: false,
            registered_boundary_exploration: true,
            coverage_scope: "full-registered-boundaries-failpoint-adapter",
            role: "Kobo-managed failure injection hooks",
        },
    ]
}

pub fn reserved_metadata_capabilities() -> &'static [MetadataBackendCapability] {
    &[
        MetadataBackendCapability {
            name: "loom",
            display_name: "Loom",
            backend_fit: "sync concurrency interleavings",
        },
        MetadataBackendCapability {
            name: "shuttle",
            display_name: "Shuttle",
            backend_fit: "async spawn/select schedule exploration",
        },
        MetadataBackendCapability {
            name: "turmoil",
            display_name: "Turmoil",
            backend_fit: "network schedule exploration metadata",
        },
        MetadataBackendCapability {
            name: "madsim",
            display_name: "Madsim",
            backend_fit: "distributed schedule exploration metadata",
        },
        MetadataBackendCapability {
            name: "proptest",
            display_name: "proptest",
            backend_fit: "input and property exploration",
        },
        MetadataBackendCapability {
            name: "failpoints",
            display_name: "failpoints",
            backend_fit: "manual failure injection points",
        },
    ]
}

pub fn execute_profile(
    profile: &str,
    seed: u64,
    semantic_trace_hash: &str,
    harness_trace_hash: &str,
    generated_rust_hash: &str,
    events: &[ScenarioEvent],
) -> Option<BackendExecution> {
    let event_count = events.len();
    let event_kinds = events
        .iter()
        .map(|event| event.kind.as_str())
        .collect::<Vec<_>>()
        .join(",");
    match profile {
        "sync" => Some(execution(
            "loom",
            "generated-user-rust",
            "generated-user-rust-loom",
            seed,
            semantic_trace_hash,
            harness_trace_hash,
            generated_rust_hash,
            event_count,
            &event_kinds,
        )),
        _ => None,
    }
}

fn execution(
    engine: &'static str,
    integration_level: &'static str,
    scenario_execution: &'static str,
    seed: u64,
    semantic_trace_hash: &str,
    harness_trace_hash: &str,
    generated_rust_hash: &str,
    event_count: usize,
    event_kinds: &str,
) -> BackendExecution {
    BackendExecution {
        engine,
        integration_level,
        scenario_execution,
        token_material: format!(
            "{engine}:{seed}:{semantic_trace_hash}:{harness_trace_hash}:{generated_rust_hash}:{event_count}:{event_kinds}"
        ),
    }
}
