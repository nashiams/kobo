import ObligationRules

namespace Kobo

def sampleTemplateAssumption : TemplateAssumption :=
  {
    templateId := "queue_delivery",
    templateVersion := "0.1",
    obligationKind := "Delivery",
    source := "built-in",
    confidence := "modeled",
    statement := "Delivery obligations are discharged, returned, transferred, canceled with policy, or escaped with ledger before loop back-edge"
  }

structure SampleTraceEvidence where
  opaqueLedgerRecorded : Bool
  deriving DecidableEq, Repr

def sampleEvidence : SampleTraceEvidence :=
  { opaqueLedgerRecorded := true }

def sampleTrace : List RuleId :=
  [
    RuleId.create,
    RuleId.split,
    RuleId.transfer,
    RuleId.discharge,
    RuleId.return,
    RuleId.cancel,
    RuleId.panic,
    RuleId.opaque
  ]

def sampleStart : Env := []

theorem sample_trace_uses_explicit_template_assumption :
    sampleTemplateAssumption.templateId = "queue_delivery"
      ∧ sampleTemplateAssumption.templateVersion = "0.1" := by
  constructor <;> rfl

theorem sample_trace_has_opaque_ledger : sampleEvidence.opaqueLedgerRecorded = true := by
  rfl

end Kobo
