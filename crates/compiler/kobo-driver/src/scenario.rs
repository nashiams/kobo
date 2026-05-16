use kobo_ir::ScenarioProgram;

use crate::pipeline::CodegenArtifacts;

#[derive(Debug, thiserror::Error)]
pub enum ScenarioBuildError {
    #[error("no #[kobo::scenario] function found")]
    MissingScenario,
}

pub fn build_scenario_program(
    artifacts: &CodegenArtifacts,
    target: &str,
    source_hash: String,
    fallback_profile: &str,
) -> Result<ScenarioProgram, ScenarioBuildError> {
    let _ = fallback_profile;
    let mut program = artifacts
        .scenario_programs
        .iter()
        .find(|program| program.target == target)
        .cloned()
        .ok_or(ScenarioBuildError::MissingScenario)?;
    program.target = target.to_owned();
    program.source_hash = source_hash;
    Ok(program)
}
