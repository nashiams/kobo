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

theorem typed_writeRuleOutputState
    (env : Env)
    (rule : RuleId)
    (id : ObligationId)
    (state : ObligationState)
    (output : RuleOutputState rule state)
    (nonempty : id ≠ "")
    (typed : typedObligationEnv env) :
    typedObligationEnv (writeState id state env) := by
  constructor
  · exact wellFormed_writeState env id state nonempty typed.left
  · intro query obligation lookup
    unfold writeState at lookup
    by_cases same : query = id
    · simp [same] at lookup
      cases lookup
      exact ruleOutputStateKnown rule state output
    · simp [same] at lookup
      exact typed.right query obligation lookup

theorem preservation_create (env next : Env) :
    Step env RuleId.create next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_create id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.create id ObligationState.owned
          RuleOutputState.createOwned nonempty typed

theorem preservation_transfer (env next : Env) :
    Step env RuleId.transfer next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_transfer id nonempty projection =>
      cases projection with
      | transferred =>
          exact
            typed_writeRuleOutputState env RuleId.transfer id ObligationState.transferred
              RuleOutputState.transferTransferred nonempty typed
      | moved =>
          exact
            typed_writeRuleOutputState env RuleId.transfer id ObligationState.moved
              RuleOutputState.transferMoved nonempty typed

theorem preservation_split (env next : Env) :
    Step env RuleId.split next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_split id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.split id ObligationState.branchUnresolved
          RuleOutputState.splitBranchUnresolved nonempty typed

theorem preservation_discharge (env next : Env) :
    Step env RuleId.discharge next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_discharge id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.discharge id ObligationState.resolved
          RuleOutputState.dischargeResolved nonempty typed

theorem preservation_return (env next : Env) :
    ModeledExitStep env ModeledExit.return next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_return => exact typed

theorem preservation_cancel (env next : Env) :
    ModeledExitStep env ModeledExit.cancel next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_cancel => exact typed

theorem preservation_panic (env next : Env) :
    ModeledExitStep env ModeledExit.panic next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_panic => exact typed

theorem preservation_error_exit (env next : Env) :
    ModeledExitStep env ModeledExit.errorExit next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_error_exit => exact typed

theorem preservation_break_exit (env next : Env) :
    ModeledExitStep env ModeledExit.breakExit next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_break_exit => exact typed

theorem preservation_opaque (env next : Env) :
    ModeledExitStep env ModeledExit.opaqueBoundary next ->
      typedObligationEnv env ->
      typedObligationEnv next := by
  intro step typed
  cases step with
  | step_opaque id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.opaque id ObligationState.escaped
          RuleOutputState.opaqueEscaped nonempty typed

theorem preservation_obligation_state (env next : Env) (rule : RuleId) :
    Step env rule next -> typedObligationEnv env -> typedObligationEnv next := by
  intro step typed
  cases step with
  | step_create id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.create id ObligationState.owned
          RuleOutputState.createOwned nonempty typed
  | step_transfer id nonempty projection =>
      cases projection with
      | transferred =>
          exact
            typed_writeRuleOutputState env RuleId.transfer id ObligationState.transferred
              RuleOutputState.transferTransferred nonempty typed
      | moved =>
          exact
            typed_writeRuleOutputState env RuleId.transfer id ObligationState.moved
              RuleOutputState.transferMoved nonempty typed
  | step_split id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.split id ObligationState.branchUnresolved
          RuleOutputState.splitBranchUnresolved nonempty typed
  | step_discharge id nonempty =>
      exact
        typed_writeRuleOutputState env RuleId.discharge id ObligationState.resolved
          RuleOutputState.dischargeResolved nonempty typed

theorem preservation_rule_output_matches_catalog
    (env next : Env)
    (rule : RuleId) :
    Step env rule next ->
      typedObligationEnv env ->
      typedObligationEnv next /\ exists state, RuleOutputState rule state := by
  intro step typed
  exact
    ⟨preservation_obligation_state env next rule step typed,
      step_output_state_matches_catalog env next rule step⟩

end Kobo
