namespace Kobo

abbrev ObligationId := String

inductive ObligationState where
  | owned
  | resolved
  | transferred
  | moved
  | branchUnresolved
  | escaped
  deriving DecidableEq, Repr

inductive RuleId where
  | create
  | transfer
  | split
  | discharge
  | return
  | cancel
  | panic
  | opaque
  deriving DecidableEq, Repr

inductive ModeledExit where
  | return
  | cancel
  | panic
  | errorExit
  | breakExit
  | opaqueBoundary
  deriving DecidableEq, Repr

inductive TemplateAssumptionSource where
  | builtIn
  | declaration
  | adapter
  | summary
  deriving DecidableEq, Repr

inductive TemplateAssumptionConfidence where
  | exact
  | modeled
  deriving DecidableEq, Repr

structure Obligation where
  id : ObligationId
  state : ObligationState
  owner : Option String
  deriving DecidableEq, Repr

abbrev Env := ObligationId -> Option Obligation

def emptyEnv : Env := fun _ => none

structure TemplateAssumption where
  templateId : String
  templateVersion : String
  obligationKind : String
  statement : String
  source : TemplateAssumptionSource
  confidence : TemplateAssumptionConfidence
  rustCertificateFieldPath : String
  leanAssumptionName : String
  deriving DecidableEq, Repr

structure TemplateAssumptionRequirement where
  templateId : String
  templateVersion : String
  obligationKind : String
  statement : String
  source : TemplateAssumptionSource
  confidence : TemplateAssumptionConfidence
  rustCertificateFieldPath : String
  leanAssumptionName : String
  deriving DecidableEq, Repr

structure CancellationEdgeEvidence where
  edgeId : String
  sourceBlock : String
  targetBlock : String
  cancelKind : String
  deriving DecidableEq, Repr

structure FutureStateObligationEvidence where
  obligationId : ObligationId
  state : ObligationState
  sourceField : String
  deriving DecidableEq, Repr

structure OpaqueLedgerEvidence where
  edgeId : String
  boundary : String
  obligationId : ObligationId
  cfgEdgeKind : String
  cfgEdgeTarget : String
  evidenceHash : String
  deriving DecidableEq, Repr

def templateAssumptionIsCurrent (assumption : TemplateAssumption) : Prop :=
  assumption.templateVersion = "0.1"

def templateAssumptionMatches
    (assumption : TemplateAssumption)
    (requirement : TemplateAssumptionRequirement) :
    Prop :=
  assumption.templateId = requirement.templateId
    /\ assumption.templateVersion = requirement.templateVersion
    /\ assumption.obligationKind = requirement.obligationKind
    /\ assumption.statement = requirement.statement
    /\ assumption.source = requirement.source
    /\ assumption.confidence = requirement.confidence
    /\ assumption.rustCertificateFieldPath = requirement.rustCertificateFieldPath
    /\ assumption.leanAssumptionName = requirement.leanAssumptionName

def cancellationEvidenceRecorded
    (cancelEvidence : CancellationEdgeEvidence)
    (futureObligation : FutureStateObligationEvidence)
    (id : ObligationId) :
    Prop :=
  cancelEvidence.edgeId ≠ ""
    /\ cancelEvidence.cancelKind = "await_cancel"
    /\ futureObligation.obligationId = id
    /\ futureObligation.sourceField = "core.async_model.future_state_obligations"

def writeState (id : ObligationId) (state : ObligationState) (env : Env) : Env :=
  fun query =>
    if query = id then
      some { id := id, state := state, owner := none }
    else
      env query

def wellFormedEnv (env : Env) : Prop :=
  forall query obligation, env query = some obligation -> obligation.id = query /\ query ≠ ""

def knownObligationState : ObligationState -> Prop
  | ObligationState.owned => True
  | ObligationState.resolved => True
  | ObligationState.transferred => True
  | ObligationState.moved => True
  | ObligationState.branchUnresolved => True
  | ObligationState.escaped => True

def typedObligationEnv (env : Env) : Prop :=
  wellFormedEnv env
    /\ forall query obligation, env query = some obligation -> knownObligationState obligation.state

def isUnresolvedLocalState : ObligationState -> Prop
  | ObligationState.owned => True
  | ObligationState.moved => True
  | ObligationState.branchUnresolved => True
  | _ => False

def noUnresolvedLocal (env : Env) : Prop :=
  forall query obligation, env query = some obligation -> isUnresolvedLocalState obligation.state -> False

def allRules : List RuleId :=
  [
    RuleId.create,
    RuleId.transfer,
    RuleId.split,
    RuleId.discharge,
    RuleId.return,
    RuleId.cancel,
    RuleId.panic,
    RuleId.opaque
  ]

inductive RuleInputState : RuleId -> ObligationState -> Prop where
  | transferOwned : RuleInputState RuleId.transfer ObligationState.owned
  | splitOwned : RuleInputState RuleId.split ObligationState.owned
  | dischargeOwned : RuleInputState RuleId.discharge ObligationState.owned
  | dischargeTransferred : RuleInputState RuleId.discharge ObligationState.transferred
  | opaqueOwned : RuleInputState RuleId.opaque ObligationState.owned
  | opaqueResolved : RuleInputState RuleId.opaque ObligationState.resolved
  | opaqueTransferred : RuleInputState RuleId.opaque ObligationState.transferred

inductive RuleOutputState : RuleId -> ObligationState -> Prop where
  | createOwned : RuleOutputState RuleId.create ObligationState.owned
  | transferTransferred : RuleOutputState RuleId.transfer ObligationState.transferred
  | transferMoved : RuleOutputState RuleId.transfer ObligationState.moved
  | splitBranchUnresolved :
      RuleOutputState RuleId.split ObligationState.branchUnresolved
  | dischargeResolved : RuleOutputState RuleId.discharge ObligationState.resolved
  | opaqueEscaped : RuleOutputState RuleId.opaque ObligationState.escaped

theorem ruleOutputStateKnown (rule : RuleId) (state : ObligationState) :
    RuleOutputState rule state -> knownObligationState state := by
  intro output
  cases output <;> trivial

def opaqueLedgerBindsCfgEdge (ledger : OpaqueLedgerEvidence) : Prop :=
  OpaqueLedgerEvidence.edgeId ledger ≠ ""
    /\ OpaqueLedgerEvidence.cfgEdgeKind ledger = "opaque_boundary"
    /\ OpaqueLedgerEvidence.cfgEdgeTarget ledger = "opaque_boundary"

end Kobo
