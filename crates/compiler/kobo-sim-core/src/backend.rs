#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendCapability {
    pub name: &'static str,
    pub executes_in_v10: bool,
    pub integration_level: &'static str,
    pub scenario_execution: &'static str,
    pub role: &'static str,
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
            executes_in_v10: true,
            integration_level: "generated-scenario",
            scenario_execution: "compiler-owned-generated-rust",
            role: "semantic/harness agreement and replay token evidence",
        },
        BackendCapability {
            name: "proptest",
            executes_in_v10: true,
            integration_level: "input-generation",
            scenario_execution: "stateful-generated-inputs",
            role: "stateful input generation for --fuzz",
        },
        BackendCapability {
            name: "loom",
            executes_in_v10: true,
            integration_level: "plumbing-smoke",
            scenario_execution: "fixed-adapter-model",
            role: "sync adapter invokes Loom with trace-bound token evidence; user generated scenarios are not executed under Loom semantics in v0.10",
        },
        BackendCapability {
            name: "shuttle",
            executes_in_v10: false,
            integration_level: "metadata-only",
            scenario_execution: "not-linked",
            role: "async backend profile metadata; external crate adapter not linked",
        },
        BackendCapability {
            name: "network-design",
            executes_in_v10: true,
            integration_level: "modeled-island",
            scenario_execution: "in-process-network-state-machine",
            role: "modeled in-process network island state machine",
        },
        BackendCapability {
            name: "failpoints",
            executes_in_v10: true,
            integration_level: "modeled-island",
            scenario_execution: "compiler-owned-failure-hooks",
            role: "compiler-owned failure injection hooks",
        },
    ]
}

pub fn execute_profile(
    profile: &str,
    seed: u64,
    semantic_trace_hash: &str,
) -> Option<BackendExecution> {
    (profile == "sync").then(|| execute_loom(seed, semantic_trace_hash))
}

fn execute_loom(seed: u64, semantic_trace_hash: &str) -> BackendExecution {
    loom::model(|| {
        use loom::sync::atomic::{AtomicUsize, Ordering};
        use loom::sync::Arc;
        use loom::thread;

        let counter = Arc::new(AtomicUsize::new(0));
        let left_counter = Arc::clone(&counter);
        let left = thread::spawn(move || {
            left_counter.fetch_add(1, Ordering::SeqCst);
        });
        let right_counter = Arc::clone(&counter);
        let right = thread::spawn(move || {
            right_counter.fetch_add(1, Ordering::SeqCst);
        });

        assert!(left.join().is_ok(), "loom left worker should not panic");
        assert!(right.join().is_ok(), "loom right worker should not panic");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    });

    BackendExecution {
        engine: "loom-plumbing-smoke",
        integration_level: "plumbing-smoke",
        scenario_execution: "fixed-adapter-model",
        token_material: format!("loom-plumbing-smoke:{seed}:{semantic_trace_hash}"),
    }
}
