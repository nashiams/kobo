import KoboCore

namespace Kobo

inductive Step : Env -> RuleId -> Env -> Prop where
  | step_create (env : Env) (id : ObligationId) (nonempty : id ≠ "") :
      Step env RuleId.create (writeState id ObligationState.owned env)
  | step_transfer (env : Env) (id : ObligationId) (nonempty : id ≠ "") :
      Step env RuleId.transfer (writeState id ObligationState.transferred env)
  | step_split
      (env : Env)
      (localId transferred : ObligationId)
      (localNonempty : localId ≠ "")
      (transferredNonempty : transferred ≠ "") :
      Step env RuleId.split
        (writeState transferred ObligationState.transferred
          (writeState localId ObligationState.owned env))
  | step_discharge (env : Env) (id : ObligationId) (nonempty : id ≠ "") :
      Step env RuleId.discharge (writeState id ObligationState.resolved env)
  | step_return (env : Env) (safe : noUnresolvedLocal env) :
      Step env RuleId.return env
  | step_cancel (env : Env) (safe : noUnresolvedLocal env) :
      Step env RuleId.cancel env
  | step_panic (env : Env) (safe : noUnresolvedLocal env) :
      Step env RuleId.panic env
  | step_opaque (env : Env) (id : ObligationId) (nonempty : id ≠ "") :
      Step env RuleId.opaque (writeState id ObligationState.escaped env)

inductive TraceAccepted : Env -> List RuleId -> Env -> Prop where
  | done (env : Env) : TraceAccepted env [] env
  | step
      (env middle final : Env)
      (rule : RuleId)
      (rest : List RuleId)
      (head : Step env rule middle)
      (tail : TraceAccepted middle rest final) :
      TraceAccepted env (rule :: rest) final

inductive ModeledExitAccepted : Env -> ModeledExit -> Prop where
  | accepted_return (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitAccepted env ModeledExit.return
  | accepted_cancel (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitAccepted env ModeledExit.cancel
  | accepted_panic (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitAccepted env ModeledExit.panic
  | accepted_opaque (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitAccepted env ModeledExit.opaqueBoundary

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
