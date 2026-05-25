import KoboCore

namespace Kobo

def requiresState (env : Env) (id : ObligationId) (state : ObligationState) : Prop :=
  exists obligation, env id = some obligation /\ obligation.state = state

def opaqueLedgerRecorded
    (ledger : OpaqueLedgerEvidence)
    (id : ObligationId)
    (boundary : String) :
    Prop :=
  OpaqueLedgerEvidence.obligationId ledger = id
    /\ OpaqueLedgerEvidence.boundary ledger = boundary
    /\ OpaqueLedgerEvidence.edgeId ledger ≠ ""
    /\ OpaqueLedgerEvidence.evidenceHash ledger ≠ ""

inductive TransferProjection where
  | transferred
  | moved
  deriving DecidableEq, Repr

def transferOutputState : TransferProjection -> ObligationState
  | TransferProjection.transferred => ObligationState.transferred
  | TransferProjection.moved => ObligationState.moved

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
      (projection : TransferProjection)
      (owned : requiresState env id ObligationState.owned) :
      Step env RuleId.transfer (writeState id (transferOutputState projection) env)
  | step_split
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (owned : requiresState env id ObligationState.owned) :
      Step env RuleId.split (writeState id ObligationState.branchUnresolved env)
  | step_discharge
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (precondition : dischargePrecondition env id) :
      Step env RuleId.discharge (writeState id ObligationState.resolved env)

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
  | step_return (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.return env
  | step_cancel (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.cancel env
  | step_panic (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.panic env
  | step_error_exit (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.errorExit env
  | step_break_exit (env : Env) (safe : noUnresolvedLocal env) :
      ModeledExitStep env ModeledExit.breakExit env
  | step_opaque
      (env : Env)
      (id : ObligationId)
      (nonempty : id ≠ "")
      (precondition : opaqueBoundaryPrecondition env id)
      (assumption : TemplateAssumption)
      (current : templateAssumptionIsCurrent assumption)
      (requirement : TemplateAssumptionRequirement)
      (matched : templateAssumptionMatches assumption requirement)
      (ledger : OpaqueLedgerEvidence)
      (recorded : opaqueLedgerRecorded ledger id "external.queue") :
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
