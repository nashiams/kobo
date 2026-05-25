import ObligationRules

namespace Kobo

theorem resolved_state_is_not_unresolved :
    isUnresolvedLocalState ObligationState.resolved -> False := by
  intro unresolved
  cases unresolved

theorem transferred_state_is_not_unresolved :
    isUnresolvedLocalState ObligationState.transferred -> False := by
  intro unresolved
  cases unresolved

theorem escaped_state_is_not_unresolved :
    isUnresolvedLocalState ObligationState.escaped -> False := by
  intro unresolved
  cases unresolved

theorem same_env_exit_cannot_drop_present_obligation
    (before after : Env)
    (exit : ModeledExit)
    (obligation : Obligation)
    (step : ModeledExitStep before exit after)
    (beforePresent : before obligation.id = some obligation)
    (afterMissing : after obligation.id = none) :
    False := by
  cases step with
  | return_preserves =>
      rw [beforePresent] at afterMissing
      contradiction
  | cancel_preserves =>
      rw [beforePresent] at afterMissing
      contradiction
  | panic_preserves =>
      rw [beforePresent] at afterMissing
      contradiction
  | opaque_records id nonempty =>
      unfold writeState at afterMissing
      by_cases same : obligation.id = id
      · simp [same] at afterMissing
      · simp [same] at afterMissing
        rw [beforePresent] at afterMissing
        contradiction

theorem no_silent_loss_on_modeled_exit
    (before after : Env)
    (exit : ModeledExit)
    (obligation : Obligation)
    (step : ModeledExitStep before exit after)
    (beforePresent : before obligation.id = some obligation)
    (afterMissing : after obligation.id = none)
    (_unresolved : isUnresolvedLocalState obligation.state) :
    False := by
  exact same_env_exit_cannot_drop_present_obligation before after exit obligation step beforePresent afterMissing

end Kobo
