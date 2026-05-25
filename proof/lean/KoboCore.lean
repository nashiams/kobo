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

def templateAssumptionIsCurrent (assumption : TemplateAssumption) : Prop :=
  assumption.templateVersion = "0.1"

def writeState (id : ObligationId) (state : ObligationState) (env : Env) : Env :=
  fun query =>
    if query = id then
      some { id := id, state := state, owner := none }
    else
      env query

def wellFormedEnv (env : Env) : Prop :=
  forall query obligation, env query = some obligation -> obligation.id = query /\ query ≠ ""

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

end Kobo
