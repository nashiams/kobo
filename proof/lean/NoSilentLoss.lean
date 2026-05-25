import ObligationRules

namespace Kobo

theorem modeled_exit_acceptance_has_no_unresolved
    (env : Env)
    (exit : ModeledExit)
    (accepted : ModeledExitAccepted env exit) :
    noUnresolvedLocal env := by
  cases accepted with
  | accepted_return safe => exact safe
  | accepted_cancel safe => exact safe
  | accepted_panic safe => exact safe
  | accepted_opaque safe => exact safe

theorem no_silent_loss_on_modeled_exit
    (env : Env)
    (exit : ModeledExit)
    (obligation : Obligation)
    (accepted : ModeledExitAccepted env exit)
    (lookup : env obligation.id = some obligation)
    (unresolved : isUnresolvedLocalState obligation.state) :
    False := by
  exact modeled_exit_acceptance_has_no_unresolved env exit accepted obligation.id obligation lookup unresolved

end Kobo
