import ObligationRules

namespace Kobo

def acceptedModeledExit (env : Env) (_exit : ModeledExit) : Prop :=
  noUnresolvedLocal env

theorem no_silent_loss_on_modeled_exit
    (env : Env)
    (exit : ModeledExit)
    (obligation : Obligation)
    (accepted : acceptedModeledExit env exit)
    (member : obligation ∈ env)
    (unresolved : isUnresolvedLocalState obligation.state) :
    False := by
  exact accepted obligation member unresolved

end Kobo
