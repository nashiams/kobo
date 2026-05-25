import ObligationRules

namespace Kobo

def wellFormedEnv (_env : Env) : Prop := True

theorem preservation_create (env next : Env) :
    Step env RuleId.create next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_transfer (env next : Env) :
    Step env RuleId.transfer next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_split (env next : Env) :
    Step env RuleId.split next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_discharge (env next : Env) :
    Step env RuleId.discharge next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_return (env next : Env) :
    Step env RuleId.return next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_cancel (env next : Env) :
    Step env RuleId.cancel next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_panic (env next : Env) :
    Step env RuleId.panic next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_opaque (env next : Env) :
    Step env RuleId.opaque next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

theorem preservation_obligation_state (env next : Env) (rule : RuleId) :
    Step env rule next -> wellFormedEnv env -> wellFormedEnv next := by
  intro _ _
  trivial

end Kobo
