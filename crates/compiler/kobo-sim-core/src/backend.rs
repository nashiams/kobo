use crate::core::{ModeledBoundary, ScenarioOperation};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendCapability {
    pub name: &'static str,
    pub display_name: &'static str,
    pub executes_in_v10: bool,
    pub integration_level: &'static str,
    pub scenario_execution: &'static str,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoomScenario {
    operation_hash: String,
    steps: Vec<LoomScenarioStep>,
    task_boundary_count: usize,
    select_branch_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoomScenarioStep {
    kind: LoomScenarioStepKind,
    label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LoomScenarioStepKind {
    CreateObligation,
    Discharge,
    MoveBinding,
    TaskBoundary,
    Select { branch_count: u32 },
    Storage,
    Network,
    Other,
}

pub fn capabilities() -> &'static [BackendCapability] {
    &[
        BackendCapability {
            name: "generated-rust-process",
            display_name: "generated-rust-process",
            executes_in_v10: true,
            integration_level: "generated-scenario",
            scenario_execution: "compiler-owned-generated-rust",
            role: "semantic/harness agreement and replay token evidence",
        },
        BackendCapability {
            name: "proptest",
            display_name: "proptest",
            executes_in_v10: true,
            integration_level: "input-generation",
            scenario_execution: "stateful-generated-inputs",
            role: "stateful input generation for --fuzz",
        },
        BackendCapability {
            name: "loom",
            display_name: "Loom",
            executes_in_v10: true,
            integration_level: "generated-scenario",
            scenario_execution: "lowered-scenario-loom",
            role:
                "sync adapter executes the compiler-owned lowered scenario state machine under Loom",
        },
        BackendCapability {
            name: "shuttle",
            display_name: "Shuttle",
            executes_in_v10: false,
            integration_level: "metadata-only",
            scenario_execution: "not-linked",
            role: "async backend profile metadata; external crate adapter not linked",
        },
        BackendCapability {
            name: "turmoil",
            display_name: "Turmoil",
            executes_in_v10: false,
            integration_level: "metadata-only",
            scenario_execution: "not-linked",
            role: "external network backend metadata; external crate adapter not linked",
        },
        BackendCapability {
            name: "madsim",
            display_name: "Madsim",
            executes_in_v10: false,
            integration_level: "metadata-only",
            scenario_execution: "not-linked",
            role: "external distributed backend metadata; external crate adapter not linked",
        },
        BackendCapability {
            name: "network-design",
            display_name: "network-design",
            executes_in_v10: true,
            integration_level: "modeled-island",
            scenario_execution: "in-process-network-state-machine",
            role: "modeled in-process network island state machine",
        },
        BackendCapability {
            name: "failpoints",
            display_name: "failpoints",
            executes_in_v10: true,
            integration_level: "modeled-island",
            scenario_execution: "compiler-owned-failure-hooks",
            role: "compiler-owned failure injection hooks",
        },
    ]
}

pub fn v085_metadata_capabilities() -> &'static [MetadataBackendCapability] {
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
    operations: &[ScenarioOperation],
) -> Option<BackendExecution> {
    (profile == "sync").then(|| execute_loom(seed, semantic_trace_hash, operations))
}

fn execute_loom(
    seed: u64,
    semantic_trace_hash: &str,
    operations: &[ScenarioOperation],
) -> BackendExecution {
    let scenario = LoomScenario::from_operations(operations);
    run_loom_scenario(scenario.clone());

    BackendExecution {
        engine: "loom-generated-scenario",
        integration_level: "generated-scenario",
        scenario_execution: "lowered-scenario-loom",
        token_material: format!(
            "loom-generated-scenario:{seed}:{semantic_trace_hash}:{}:{}:{}:{}",
            scenario.operation_hash,
            scenario.steps.len(),
            scenario.task_boundary_count,
            scenario.select_branch_count
        ),
    }
}

impl LoomScenario {
    fn from_operations(operations: &[ScenarioOperation]) -> Self {
        let steps = operations
            .iter()
            .map(LoomScenarioStep::from_operation)
            .collect::<Vec<_>>();
        let task_boundary_count = steps
            .iter()
            .filter(|step| matches!(step.kind, LoomScenarioStepKind::TaskBoundary))
            .count();
        let select_branch_count = steps
            .iter()
            .filter_map(|step| match step.kind {
                LoomScenarioStepKind::Select { branch_count } => Some(branch_count),
                _ => None,
            })
            .sum();
        Self {
            operation_hash: crate::digest::operations_hash(operations),
            steps,
            task_boundary_count,
            select_branch_count,
        }
    }
}

impl LoomScenarioStep {
    fn from_operation(operation: &ScenarioOperation) -> Self {
        match operation {
            ScenarioOperation::CreateObligation { binding, .. } => Self {
                kind: LoomScenarioStepKind::CreateObligation,
                label: binding.clone(),
            },
            ScenarioOperation::Discharge { binding, .. } => Self {
                kind: LoomScenarioStepKind::Discharge,
                label: binding.clone(),
            },
            ScenarioOperation::MoveBinding { binding, .. } => Self {
                kind: LoomScenarioStepKind::MoveBinding,
                label: binding.clone(),
            },
            ScenarioOperation::ModeledEffect {
                boundary: ModeledBoundary::WardTask,
                ..
            } => Self {
                kind: LoomScenarioStepKind::TaskBoundary,
                label: "ward.task".to_owned(),
            },
            ScenarioOperation::Select { branch_count, .. } => Self {
                kind: LoomScenarioStepKind::Select {
                    branch_count: *branch_count,
                },
                label: "select".to_owned(),
            },
            ScenarioOperation::StorageEvent { action, .. } => Self {
                kind: LoomScenarioStepKind::Storage,
                label: action.clone(),
            },
            ScenarioOperation::NetworkEvent { action, .. } => Self {
                kind: LoomScenarioStepKind::Network,
                label: action.clone(),
            },
            _ => Self {
                kind: LoomScenarioStepKind::Other,
                label: "operation".to_owned(),
            },
        }
    }
}

fn run_loom_scenario(scenario: LoomScenario) {
    loom::model(move || {
        use loom::sync::atomic::{AtomicUsize, Ordering};
        use loom::sync::Arc;
        use loom::thread;

        let steps = Arc::new(scenario.steps.clone());
        let live_obligations = Arc::new(AtomicUsize::new(0));
        let completed_operations = Arc::new(AtomicUsize::new(0));
        let scheduler_observations = Arc::new(AtomicUsize::new(0));

        let executor_steps = Arc::clone(&steps);
        let executor_live = Arc::clone(&live_obligations);
        let executor_completed = Arc::clone(&completed_operations);
        let executor_scheduler = Arc::clone(&scheduler_observations);
        let executor = thread::spawn(move || {
            for step in executor_steps.iter() {
                execute_loom_step(
                    step,
                    &executor_live,
                    &executor_completed,
                    &executor_scheduler,
                );
            }
        });

        let observer_steps = Arc::clone(&steps);
        let observer_live = Arc::clone(&live_obligations);
        let observer_scheduler = Arc::clone(&scheduler_observations);
        let observer = thread::spawn(move || {
            for step in observer_steps.iter() {
                if matches!(
                    step.kind,
                    LoomScenarioStepKind::TaskBoundary | LoomScenarioStepKind::Select { .. }
                ) {
                    thread::yield_now();
                    let _live_count = observer_live.load(Ordering::SeqCst);
                    observer_scheduler.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        assert!(
            executor.join().is_ok(),
            "loom scenario executor should not panic"
        );
        assert!(
            observer.join().is_ok(),
            "loom scenario observer should not panic"
        );
        assert_eq!(
            completed_operations.load(Ordering::SeqCst),
            steps.len(),
            "loom scenario must execute every lowered operation"
        );
    });
}

fn execute_loom_step(
    step: &LoomScenarioStep,
    live_obligations: &loom::sync::atomic::AtomicUsize,
    completed_operations: &loom::sync::atomic::AtomicUsize,
    scheduler_observations: &loom::sync::atomic::AtomicUsize,
) {
    use loom::sync::atomic::Ordering;
    use loom::thread;

    match step.kind {
        LoomScenarioStepKind::CreateObligation => {
            live_obligations.fetch_add(1, Ordering::SeqCst);
        }
        LoomScenarioStepKind::Discharge => {
            decrement_if_live(live_obligations);
        }
        LoomScenarioStepKind::MoveBinding => {}
        LoomScenarioStepKind::TaskBoundary => {
            thread::yield_now();
            scheduler_observations.fetch_add(1, Ordering::SeqCst);
        }
        LoomScenarioStepKind::Select { branch_count } => {
            for _ in 0..branch_count {
                thread::yield_now();
                scheduler_observations.fetch_add(1, Ordering::SeqCst);
            }
        }
        LoomScenarioStepKind::Storage
        | LoomScenarioStepKind::Network
        | LoomScenarioStepKind::Other => {}
    }
    completed_operations.fetch_add(1, Ordering::SeqCst);
}

fn decrement_if_live(counter: &loom::sync::atomic::AtomicUsize) {
    use loom::sync::atomic::Ordering;

    let mut current = counter.load(Ordering::SeqCst);
    while current > 0 {
        match counter.compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}
