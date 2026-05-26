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

inductive PermittedAccountingRule : RuleId -> Prop where
  | transfer : PermittedAccountingRule RuleId.transfer
  | discharge : PermittedAccountingRule RuleId.discharge
  | opaque : PermittedAccountingRule RuleId.opaque

inductive ExitAccountingRule : ModeledExit -> RuleId -> Prop where
  | return : ExitAccountingRule ModeledExit.return RuleId.return
  | cancel : ExitAccountingRule ModeledExit.cancel RuleId.cancel
  | panic : ExitAccountingRule ModeledExit.panic RuleId.panic
  | opaque : ExitAccountingRule ModeledExit.opaqueBoundary RuleId.opaque

inductive SilentLossOnModeledExit
    (before after : Env)
    (exit : ModeledExit)
    (obligation : Obligation) :
    Prop where
  | missing
      (beforePresent : before obligation.id = some obligation)
      (afterMissing : after obligation.id = none)
      (unresolved : isUnresolvedLocalState obligation.state) :
      SilentLossOnModeledExit before after exit obligation
  | unaccountedStateChange
      (beforePresent : before obligation.id = some obligation)
      (afterObligation : Obligation)
      (afterPresent : after obligation.id = some afterObligation)
      (unresolved : isUnresolvedLocalState obligation.state)
      (changed : afterObligation.state ≠ obligation.state)
      (afterNotUnresolved : isUnresolvedLocalState afterObligation.state -> False)
      (noAccounting : forall rule, ExitAccountingRule exit rule -> False) :
      SilentLossOnModeledExit before after exit obligation

theorem unresolved_local_state_change_requires_accounting
    (before after : Env)
    (exit : ModeledExit)
    (obligation afterObligation : Obligation)
    (step : ModeledExitStep before exit after)
    (beforePresent : before obligation.id = some obligation)
    (afterPresent : after obligation.id = some afterObligation)
    (unresolved : isUnresolvedLocalState obligation.state)
    (_changed : afterObligation.state ≠ obligation.state) :
    exists rule, ExitAccountingRule exit rule := by
  cases step with
  | step_return safe =>
      exact False.elim (safe obligation.id obligation beforePresent unresolved)
  | step_cancel safe id cancelEvidence futureObligation recorded =>
      exact False.elim (safe obligation.id obligation beforePresent unresolved)
  | step_panic safe =>
      exact False.elim (safe obligation.id obligation beforePresent unresolved)
  | step_error_exit safe =>
      exact False.elim (safe obligation.id obligation beforePresent unresolved)
  | step_break_exit safe =>
      exact False.elim (safe obligation.id obligation beforePresent unresolved)
  | step_opaque id nonempty precondition assumption current requirement matched ledger recorded =>
      exact ⟨RuleId.opaque, ExitAccountingRule.opaque⟩

theorem opaque_exit_change_has_ledger_accounting
    (before after : Env)
    (step : ModeledExitStep before ModeledExit.opaqueBoundary after) :
    exists id ledger, opaqueLedgerRecorded ledger id "external.queue" := by
  cases step with
  | step_opaque id _nonempty _precondition _assumption _current _requirement _matched ledger recorded =>
      exact ⟨id, ledger, recorded⟩

theorem opaque_exit_change_uses_ledger_cfg_edge
    (before after : Env)
    (step : ModeledExitStep before ModeledExit.opaqueBoundary after) :
    exists id ledger,
      opaqueLedgerRecorded ledger id "external.queue"
        /\ opaqueLedgerBindsCfgEdge ledger := by
  cases step with
  | step_opaque id _nonempty _precondition _assumption _current _requirement _matched ledger recorded =>
      exact ⟨id, ledger, recorded, recorded.right.right.right⟩

theorem return_exit_rejects_unresolved_before
    (before after : Env)
    (obligation : Obligation)
    (step : ModeledExitStep before ModeledExit.return after)
    (beforePresent : before obligation.id = some obligation)
    (unresolved : isUnresolvedLocalState obligation.state) :
    False := by
  cases step with
  | step_return safe =>
      exact safe obligation.id obligation beforePresent unresolved

theorem panic_exit_rejects_unresolved_before
    (before after : Env)
    (obligation : Obligation)
    (step : ModeledExitStep before ModeledExit.panic after)
    (beforePresent : before obligation.id = some obligation)
    (unresolved : isUnresolvedLocalState obligation.state) :
    False := by
  cases step with
  | step_panic safe =>
      exact safe obligation.id obligation beforePresent unresolved

theorem cancel_exit_rejects_unresolved_before
    (before after : Env)
    (obligation : Obligation)
    (step : ModeledExitStep before ModeledExit.cancel after)
    (beforePresent : before obligation.id = some obligation)
    (unresolved : isUnresolvedLocalState obligation.state) :
    False := by
  cases step with
  | step_cancel safe =>
      exact safe obligation.id obligation beforePresent unresolved

theorem same_env_exit_cannot_drop_present_obligation
    (before after : Env)
    (exit : ModeledExit)
    (obligation : Obligation)
    (step : ModeledExitStep before exit after)
    (beforePresent : before obligation.id = some obligation)
    (afterMissing : after obligation.id = none) :
    False := by
  cases step with
  | step_return =>
      rw [beforePresent] at afterMissing
      contradiction
  | step_cancel =>
      rw [beforePresent] at afterMissing
      contradiction
  | step_panic =>
      rw [beforePresent] at afterMissing
      contradiction
  | step_error_exit =>
      rw [beforePresent] at afterMissing
      contradiction
  | step_break_exit =>
      rw [beforePresent] at afterMissing
      contradiction
  | step_opaque id nonempty =>
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
    (loss : SilentLossOnModeledExit before after exit obligation) :
    False := by
  cases loss with
  | missing beforePresent afterMissing unresolved =>
      cases step with
      | step_return safe =>
          exact safe obligation.id obligation beforePresent unresolved
      | step_cancel safe id cancelEvidence futureObligation recorded =>
          exact safe obligation.id obligation beforePresent unresolved
      | step_panic safe =>
          exact safe obligation.id obligation beforePresent unresolved
      | step_error_exit safe =>
          exact safe obligation.id obligation beforePresent unresolved
      | step_break_exit safe =>
          exact safe obligation.id obligation beforePresent unresolved
      | step_opaque id nonempty =>
          unfold writeState at afterMissing
          by_cases same : obligation.id = id
          · simp [same] at afterMissing
          · simp [same] at afterMissing
            rw [beforePresent] at afterMissing
            contradiction
  | unaccountedStateChange beforePresent afterObligation afterPresent unresolved changed _afterNotUnresolved noAccounting =>
      have accounting :=
        unresolved_local_state_change_requires_accounting
          before after exit obligation afterObligation step beforePresent afterPresent unresolved changed
      cases accounting with
      | intro rule accounted =>
          exact noAccounting rule accounted

end Kobo
