import ObligationRules
import NoSilentLoss

namespace Kobo

def delivery : ObligationId := "delivery"
def splitDelivery : ObligationId := "delivery_transferred"
def opaqueDelivery : ObligationId := "delivery_opaque"

theorem delivery_nonempty : delivery ≠ "" := by
  decide

theorem split_delivery_nonempty : splitDelivery ≠ "" := by
  decide

theorem opaque_delivery_nonempty : opaqueDelivery ≠ "" := by
  decide

def sampleTemplateAssumption : TemplateAssumption :=
  {
    templateId := "queue_delivery",
    templateVersion := "0.1",
    obligationKind := "Delivery",
    statement := "Delivery obligations are discharged, returned, transferred, canceled with policy, or escaped with ledger before loop back-edge",
    source := TemplateAssumptionSource.builtIn,
    confidence := TemplateAssumptionConfidence.modeled,
    rustCertificateFieldPath := "template_schemas",
    leanAssumptionName := "queue_delivery_assumption"
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

def sampleStart : Env := emptyEnv
def sampleAfterCreate : Env := writeState delivery ObligationState.owned sampleStart
def sampleAfterSplit : Env :=
  writeState splitDelivery ObligationState.transferred
    (writeState delivery ObligationState.owned sampleAfterCreate)
def sampleAfterTransfer : Env :=
  writeState splitDelivery ObligationState.transferred sampleAfterSplit
def sampleAfterDischarge : Env :=
  writeState delivery ObligationState.resolved sampleAfterTransfer
def sampleAfterReturn : Env := sampleAfterDischarge
def sampleAfterCancel : Env := sampleAfterReturn
def sampleAfterPanic : Env := sampleAfterCancel
def sampleEnd : Env := writeState opaqueDelivery ObligationState.escaped sampleAfterPanic

theorem template_assumption_is_current :
    templateAssumptionIsCurrent sampleTemplateAssumption := by
  rfl

theorem stale_template_assumption_is_rejected
    (assumption : TemplateAssumption)
    (_sameId : assumption.templateId = "queue_delivery")
    (stale : assumption.templateVersion ≠ "0.1") :
    templateAssumptionIsCurrent assumption -> False := by
  intro current
  exact stale current

theorem sample_trace_has_opaque_ledger : sampleEvidence.opaqueLedgerRecorded = true := by
  rfl

theorem sample_after_discharge_has_no_unresolved :
    noUnresolvedLocal sampleAfterDischarge := by
  intro query obligation lookup unresolved
  unfold sampleAfterDischarge sampleAfterTransfer sampleAfterSplit sampleAfterCreate sampleStart emptyEnv writeState at lookup
  by_cases queryDelivery : query = delivery
  · simp [queryDelivery] at lookup
    cases lookup
    cases unresolved
  · by_cases querySplit : query = splitDelivery
    · simp [queryDelivery, querySplit] at lookup
      cases lookup
      cases unresolved
    · simp [queryDelivery, querySplit] at lookup

theorem sample_after_return_has_no_unresolved :
    noUnresolvedLocal sampleAfterReturn := by
  exact sample_after_discharge_has_no_unresolved

theorem sample_after_cancel_has_no_unresolved :
    noUnresolvedLocal sampleAfterCancel := by
  exact sample_after_return_has_no_unresolved

theorem sample_after_panic_has_no_unresolved :
    noUnresolvedLocal sampleAfterPanic := by
  exact sample_after_cancel_has_no_unresolved

theorem sample_trace_accepted :
    TraceAccepted sampleStart sampleTrace sampleEnd := by
  apply TraceAccepted.step
  · exact Step.step_create sampleStart delivery delivery_nonempty
  · apply TraceAccepted.step
    · exact Step.step_split sampleAfterCreate delivery splitDelivery delivery_nonempty split_delivery_nonempty
    · apply TraceAccepted.step
      · exact Step.step_transfer sampleAfterSplit splitDelivery split_delivery_nonempty
      · apply TraceAccepted.step
        · exact Step.step_discharge sampleAfterTransfer delivery delivery_nonempty
        · apply TraceAccepted.step
          · exact Step.step_return sampleAfterDischarge sample_after_discharge_has_no_unresolved
          · apply TraceAccepted.step
            · exact Step.step_cancel sampleAfterReturn sample_after_return_has_no_unresolved
            · apply TraceAccepted.step
              · exact Step.step_panic sampleAfterCancel sample_after_cancel_has_no_unresolved
              · apply TraceAccepted.step
                · exact Step.step_opaque sampleAfterPanic opaqueDelivery opaque_delivery_nonempty
                · exact TraceAccepted.done sampleEnd

end Kobo
