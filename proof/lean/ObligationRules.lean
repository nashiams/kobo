import KoboCore

namespace Kobo

def requiresState (env : Env) (id : ObligationId) (state : ObligationState) : Prop :=
  exists obligation, env id = some obligation /\ obligation.state = state

def opaqueLedgerRecorded (ledger : ObligationId -> Bool) (id : ObligationId) : Prop :=
  ledger id = true

def dischargePrecondition (env : Env) (id : ObligationId) : Prop :=
  requiresState env id ObligationState.owned
    \/ requiresState env id ObligationState.transferred

def opaqueBoundaryPrecondition (env : Env) (id : ObligationId) : Prop :=
  requiresState env id ObligationState.owned
    \/ requiresState env id ObligationState.transferred
    \/ requiresState env id ObligationState.resolved

inductive Step : Env -> RuleId -> Env -> Prop where
  | step_create (env : Env) (id : ObligationId) (nonempty : id ≠ "") :
      Step env RuleId.create (writeState id ObligationState.owned env)
  | step_transfer
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (owned : requiresState env id ObligationState.owned) :
      Step env RuleId.transfer (writeState id ObligationState.transferred env)
  | step_split
      (env : Env)
      (localId transferred : ObligationId)
      (localNonempty : localId ≠ "")
      (transferredNonempty : transferred ≠ "")
      (owned : requiresState env localId ObligationState.owned) :
      Step env RuleId.split
        (writeState transferred ObligationState.transferred
          (writeState localId ObligationState.owned env))
  | step_discharge
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (precondition : dischargePrecondition env id) :
      Step env RuleId.discharge (writeState id ObligationState.resolved env)
  | step_return (env : Env) (safe : noUnresolvedLocal env) :
      Step env RuleId.return env
  | step_cancel (env : Env) (safe : noUnresolvedLocal env) :
      Step env RuleId.cancel env
  | step_panic (env : Env) (safe : noUnresolvedLocal env) :
      Step env RuleId.panic env
  | step_opaque
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (precondition : opaqueBoundaryPrecondition env id)
      (assumption : TemplateAssumption)
      (current : templateAssumptionIsCurrent assumption)
      (ledger : ObligationId -> Bool)
      (recorded : opaqueLedgerRecorded ledger id) :
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

inductive ModeledExitStep : Env -> ModeledExit -> Env -> Prop where
  | return_preserves (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.return env
  | cancel_preserves (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.cancel env
  | panic_preserves (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.panic env
  | opaque_records
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (precondition : opaqueBoundaryPrecondition env id)
      (assumption : TemplateAssumption)
      (current : templateAssumptionIsCurrent assumption)
      (ledger : ObligationId -> Bool)
      (recorded : opaqueLedgerRecorded ledger id) :
      ModeledExitStep env ModeledExit.opaqueBoundary (writeState id ObligationState.escaped env)

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
