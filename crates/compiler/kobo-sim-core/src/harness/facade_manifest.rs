use kobo_ir::{ScenarioBoundaryPolicy, ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

pub(super) fn harness_facades(program: &ScenarioProgram) -> Vec<String> {
    let mut facades = Vec::new();
    for operation in &program.operations {
        match &operation.kind {
            ScenarioOpKind::ModeledEffect { boundary } => match boundary {
                ScenarioModeledBoundary::WardTime => facades.push("time-facade".to_owned()),
                ScenarioModeledBoundary::WardRandom => facades.push("random-facade".to_owned()),
                ScenarioModeledBoundary::WardTask => {
                    facades.push("scheduler-task-facade".to_owned());
                    facades.push("tokio-spawn-facade".to_owned());
                }
                ScenarioModeledBoundary::WardTaskLocal => {
                    facades.push("scheduler-task-local-facade".to_owned());
                    facades.push("tokio-spawn-local-facade".to_owned());
                }
            },
            ScenarioOpKind::StorageEvent { .. } => {
                facades.push("storage-filesystem-facade".to_owned());
            }
            ScenarioOpKind::NetworkEvent { .. } => {
                facades.push("network-loopback-facade".to_owned());
            }
            ScenarioOpKind::ExternalBoundary {
                crate_name, policy, ..
            } if is_replay_owned_boundary(policy) => {
                facades.push(format!(
                    "external-boundary-{}-facade:{crate_name}",
                    policy.as_str()
                ));
            }
            _ => {}
        }
    }
    facades.sort();
    facades.dedup();
    facades
}

pub(super) fn is_replay_owned_boundary(policy: &ScenarioBoundaryPolicy) -> bool {
    matches!(
        policy,
        ScenarioBoundaryPolicy::Model
            | ScenarioBoundaryPolicy::Record
            | ScenarioBoundaryPolicy::Stub
    )
}

pub(super) fn is_rust_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
