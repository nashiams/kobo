import KoboCore

namespace Kobo

def writeState (id : ObligationId) (state : ObligationState) (env : Env) : Env :=
  { id := id, state := state, owner := none } :: env

inductive Step : Env -> RuleId -> Env -> Prop where
  | step_create (env : Env) (id : ObligationId) :
      Step env RuleId.create (writeState id ObligationState.owned env)
  | step_transfer (env : Env) (id : ObligationId) :
      Step env RuleId.transfer (writeState id ObligationState.transferred env)
  | step_split (env : Env) (local transferred : ObligationId) :
      Step env RuleId.split
        (writeState transferred ObligationState.transferred
          (writeState local ObligationState.owned env))
  | step_discharge (env : Env) (id : ObligationId) :
      Step env RuleId.discharge (writeState id ObligationState.resolved env)
  | step_return (env : Env) (h : noUnresolvedLocal env) :
      Step env RuleId.return env
  | step_cancel (env : Env) (h : noUnresolvedLocal env) :
      Step env RuleId.cancel env
  | step_panic (env : Env) (h : noUnresolvedLocal env) :
      Step env RuleId.panic env
  | step_opaque (env : Env) (id : ObligationId) :
      Step env RuleId.opaque (writeState id ObligationState.escaped env)

def ruleName : RuleId -> String
  | RuleId.create => "create"
  | RuleId.transfer => "transfer"
  | RuleId.split => "split"
  | RuleId.discharge => "discharge"
  | RuleId.return => "return"
  | RuleId.cancel => "cancel"
  | RuleId.panic => "panic"
  | RuleId.opaque => "opaque"

end Kobo
