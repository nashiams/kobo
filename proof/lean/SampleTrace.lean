import ObligationRules
import NoSilentLoss

namespace Kobo

def delivery : ObligationId := "delivery"

theorem delivery_nonempty : delivery ≠ "" := by
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

def sampleLedger : ObligationId -> Bool :=
  fun query => if query = delivery then true else false

def sampleTrace : List RuleId :=
  [RuleId.create, RuleId.transfer, RuleId.discharge, RuleId.return, RuleId.panic, RuleId.opaque]

def sampleStart : Env := emptyEnv
def sampleAfterCreate : Env := writeState delivery ObligationState.owned sampleStart
def sampleAfterTransfer : Env :=
  writeState delivery ObligationState.transferred sampleAfterCreate
def sampleAfterDischarge : Env :=
  writeState delivery ObligationState.resolved sampleAfterTransfer
def sampleAfterReturn : Env := sampleAfterDischarge
def sampleAfterPanic : Env := sampleAfterReturn
def sampleEnd : Env := writeState delivery ObligationState.escaped sampleAfterPanic

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

theorem sample_opaque_ledger_recorded :
    opaqueLedgerRecorded sampleLedger delivery := by
  unfold opaqueLedgerRecorded sampleLedger
  simp [delivery]

theorem sample_after_create_has_owned_delivery :
    requiresState sampleAfterCreate delivery ObligationState.owned := by
  refine ⟨{ id := delivery, state := ObligationState.owned, owner := none }, ?_⟩
  constructor
  · simp [sampleAfterCreate, sampleStart, emptyEnv, writeState, delivery]
  · rfl

theorem sample_after_transfer_has_transferred_delivery :
    requiresState sampleAfterTransfer delivery ObligationState.transferred := by
  refine ⟨{ id := delivery, state := ObligationState.transferred, owner := none }, ?_⟩
  constructor
  · simp [sampleAfterTransfer, writeState, delivery]
  · rfl

theorem sample_after_discharge_has_resolved_delivery :
    requiresState sampleAfterDischarge delivery ObligationState.resolved := by
  refine ⟨{ id := delivery, state := ObligationState.resolved, owner := none }, ?_⟩
  constructor
  · simp [sampleAfterDischarge, writeState, delivery]
  · rfl

theorem sample_after_transfer_can_discharge :
    dischargePrecondition sampleAfterTransfer delivery := by
  exact Or.inr sample_after_transfer_has_transferred_delivery

theorem sample_after_panic_can_cross_opaque_boundary :
    opaqueBoundaryPrecondition sampleAfterPanic delivery := by
  exact Or.inr (Or.inr sample_after_discharge_has_resolved_delivery)

theorem sample_after_discharge_has_no_unresolved :
    noUnresolvedLocal sampleAfterDischarge := by
  intro query obligation lookup unresolved
  unfold sampleAfterDischarge sampleAfterTransfer sampleAfterCreate sampleStart emptyEnv writeState at lookup
  by_cases queryDelivery : query = delivery
  · simp [queryDelivery] at lookup
    cases lookup
    cases unresolved
  · simp [queryDelivery] at lookup

theorem sample_after_return_has_no_unresolved :
    noUnresolvedLocal sampleAfterReturn := by
  exact sample_after_discharge_has_no_unresolved

theorem sample_after_panic_has_no_unresolved :
    noUnresolvedLocal sampleAfterPanic := by
  exact sample_after_return_has_no_unresolved

theorem sample_trace_accepted :
    TraceAccepted sampleStart sampleTrace sampleEnd := by
  apply TraceAccepted.step
  · exact Step.step_create sampleStart delivery delivery_nonempty
  · apply TraceAccepted.step
    · exact
        Step.step_transfer
          sampleAfterCreate
          delivery
          delivery_nonempty
          sample_after_create_has_owned_delivery
    · apply TraceAccepted.step
      · exact
          Step.step_discharge
            sampleAfterTransfer
            delivery
            delivery_nonempty
            sample_after_transfer_can_discharge
      · apply TraceAccepted.step
        · exact Step.step_return sampleAfterDischarge sample_after_discharge_has_no_unresolved
        · apply TraceAccepted.step
          · exact Step.step_panic sampleAfterReturn sample_after_return_has_no_unresolved
          · apply TraceAccepted.step
            · exact
                Step.step_opaque
                  sampleAfterPanic
                  delivery
                  delivery_nonempty
                  sample_after_panic_can_cross_opaque_boundary
                  sampleTemplateAssumption
                  template_assumption_is_current
                  sampleLedger
                  sample_opaque_ledger_recorded
            · exact TraceAccepted.done sampleEnd

end Kobo
