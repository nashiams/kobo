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

structure Obligation where
  id : ObligationId
  state : ObligationState
  owner : Option String
  deriving DecidableEq, Repr

abbrev Env := List Obligation

structure TemplateAssumption where
  templateId : String
  templateVersion : String
  obligationKind : String
  source : String
  confidence : String
  statement : String
  deriving DecidableEq, Repr

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

def isUnresolvedLocalState : ObligationState -> Prop
  | ObligationState.owned => True
  | ObligationState.moved => True
  | ObligationState.branchUnresolved => True
  | _ => False

def noUnresolvedLocal (env : Env) : Prop :=
  ∀ obligation, obligation ∈ env -> isUnresolvedLocalState obligation.state -> False

end Kobo
