use super::FullDepthRun;
pub(super) fn runtime_boundary_evidence(
    run: &FullDepthRun,
) -> Vec<kobo_driver::proof::RuntimeBoundaryEvidence> {
    run.boundary_decisions
        .iter()
        .map(|decision| kobo_driver::proof::RuntimeBoundaryEvidence {
            boundary: decision.crate_name.clone(),
            policy: proof_boundary_policy(&decision.policy),
            has_recorded_io: decision.recorded_io.is_some(),
        })
        .collect()
}

fn proof_boundary_policy(
    policy: &kobo_sim_core::BoundaryPolicyChoice,
) -> kobo_ir::ScenarioBoundaryPolicy {
    match policy {
        kobo_sim_core::BoundaryPolicyChoice::Typed => kobo_ir::ScenarioBoundaryPolicy::Typed,
        kobo_sim_core::BoundaryPolicyChoice::Model => kobo_ir::ScenarioBoundaryPolicy::Model,
        kobo_sim_core::BoundaryPolicyChoice::Record => kobo_ir::ScenarioBoundaryPolicy::Record,
        kobo_sim_core::BoundaryPolicyChoice::Activity => kobo_ir::ScenarioBoundaryPolicy::Activity,
        kobo_sim_core::BoundaryPolicyChoice::Stub => kobo_ir::ScenarioBoundaryPolicy::Stub,
        kobo_sim_core::BoundaryPolicyChoice::Outside => kobo_ir::ScenarioBoundaryPolicy::Outside,
        kobo_sim_core::BoundaryPolicyChoice::Opaque => kobo_ir::ScenarioBoundaryPolicy::Opaque,
        kobo_sim_core::BoundaryPolicyChoice::Debt => kobo_ir::ScenarioBoundaryPolicy::Debt,
        kobo_sim_core::BoundaryPolicyChoice::Unselected => {
            kobo_ir::ScenarioBoundaryPolicy::Unselected
        }
    }
}
