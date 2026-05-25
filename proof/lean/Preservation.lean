import ObligationRules

namespace Kobo

theorem wellFormed_writeState
    (env : Env)
    (id : ObligationId)
    (state : ObligationState)
    (nonempty : id ≠ "")
    (wf : wellFormedEnv env) :
    wellFormedEnv (writeState id state env) := by
  intro query obligation lookup
  unfold writeState at lookup
  by_cases same : query = id
  · simp [same] at lookup
    cases lookup
    constructor
    · exact same.symm
    · rw [same]
      exact nonempty
  · simp [same] at lookup
    exact wf query obligation lookup

theorem preservation_create (env next : Env) :
    Step env RuleId.create next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_create id nonempty =>
      exact wellFormed_writeState env id ObligationState.owned nonempty wf

theorem preservation_transfer (env next : Env) :
    Step env RuleId.transfer next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_transfer id nonempty =>
      exact wellFormed_writeState env id ObligationState.transferred nonempty wf

theorem preservation_split (env next : Env) :
    Step env RuleId.split next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_split localId transferred localNonempty transferredNonempty =>
      exact
        wellFormed_writeState
          (writeState localId ObligationState.owned env)
          transferred
          ObligationState.transferred
          transferredNonempty
          (wellFormed_writeState env localId ObligationState.owned localNonempty wf)

theorem preservation_discharge (env next : Env) :
    Step env RuleId.discharge next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_discharge id nonempty =>
      exact wellFormed_writeState env id ObligationState.resolved nonempty wf

theorem preservation_return (env next : Env) :
    Step env RuleId.return next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_return _ => exact wf

theorem preservation_cancel (env next : Env) :
    Step env RuleId.cancel next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_cancel _ => exact wf

theorem preservation_panic (env next : Env) :
    Step env RuleId.panic next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_panic _ => exact wf

theorem preservation_opaque (env next : Env) :
    Step env RuleId.opaque next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_opaque id nonempty =>
      exact wellFormed_writeState env id ObligationState.escaped nonempty wf

theorem preservation_obligation_state (env next : Env) (rule : RuleId) :
    Step env rule next -> wellFormedEnv env -> wellFormedEnv next := by
  intro step wf
  cases step with
  | step_create id nonempty =>
      exact wellFormed_writeState env id ObligationState.owned nonempty wf
  | step_transfer id nonempty =>
      exact wellFormed_writeState env id ObligationState.transferred nonempty wf
  | step_split localId transferred localNonempty transferredNonempty =>
      exact
        wellFormed_writeState
          (writeState localId ObligationState.owned env)
          transferred
          ObligationState.transferred
          transferredNonempty
          (wellFormed_writeState env localId ObligationState.owned localNonempty wf)
  | step_discharge id nonempty =>
      exact wellFormed_writeState env id ObligationState.resolved nonempty wf
  | step_return _ => exact wf
  | step_cancel _ => exact wf
  | step_panic _ => exact wf
  | step_opaque id nonempty =>
      exact wellFormed_writeState env id ObligationState.escaped nonempty wf

end Kobo
