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

def sampleTemplateRequirement : TemplateAssumptionRequirement :=
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

def sampleLedger : OpaqueLedgerEvidence :=
  {
    edgeId := "edge-proof_case-bb2-opaque",
    boundary := "external.queue",
    obligationId := delivery,
    evidenceHash := "7ef0a4d8e1c52a63"
  }

def sampleTrace : List RuleId :=
  [RuleId.create, RuleId.transfer, RuleId.discharge, RuleId.return, RuleId.panic, RuleId.opaque]

def sampleObligationTrace : List RuleId :=
  [RuleId.create, RuleId.transfer, RuleId.discharge]

def sampleModeledExitTrace : List RuleId :=
  [RuleId.return, RuleId.panic, RuleId.opaque]

def sampleStart : Env := emptyEnv
def sampleAfterCreate : Env := writeState delivery ObligationState.owned sampleStart
def sampleAfterTransfer : Env :=
  writeState delivery ObligationState.transferred sampleAfterCreate
def sampleAfterDischarge : Env :=
  writeState delivery ObligationState.resolved sampleAfterTransfer
def sampleAfterReturn : Env := sampleAfterDischarge
def sampleAfterPanic : Env := sampleAfterDischarge
def sampleEnd : Env := writeState delivery ObligationState.escaped sampleAfterDischarge

theorem template_assumption_is_current :
    templateAssumptionIsCurrent sampleTemplateAssumption := by
  rfl

theorem sample_template_assumption_matches :
    templateAssumptionMatches sampleTemplateAssumption sampleTemplateRequirement := by
  constructor
  · rfl
  · constructor
    · rfl
    · constructor
      · rfl
      · constructor
        · rfl
        · constructor
          · rfl
          · constructor
            · rfl
            · constructor
              · rfl
              · constructor
                · rfl
                · rfl

theorem sample_template_requirement_id :
    sampleTemplateRequirement.templateId = "queue_delivery" := by
  rfl

theorem sample_template_requirement_kind :
    sampleTemplateRequirement.obligationKind = "Delivery" := by
  rfl

theorem sample_template_requirement_statement :
    sampleTemplateRequirement.statement = "Delivery obligations are discharged, returned, transferred, canceled with policy, or escaped with ledger before loop back-edge" := by
  rfl

theorem sample_template_requirement_source :
    sampleTemplateRequirement.source = TemplateAssumptionSource.builtIn := by
  rfl

theorem sample_template_requirement_confidence :
    sampleTemplateRequirement.confidence = TemplateAssumptionConfidence.modeled := by
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
    opaqueLedgerRecorded sampleLedger delivery "external.queue" := by
  repeat constructor <;> decide

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

theorem sample_obligation_trace_accepted :
    TraceAccepted sampleStart sampleObligationTrace sampleAfterDischarge := by
  apply TraceAccepted.step
  · exact Step.step_create sampleStart delivery delivery_nonempty
  · apply TraceAccepted.step
    · exact
        Step.step_transfer
          sampleAfterCreate
          delivery
          delivery_nonempty
          TransferProjection.transferred
          sample_after_create_has_owned_delivery
    · apply TraceAccepted.step
      · exact
          Step.step_discharge
            sampleAfterTransfer
            delivery
            delivery_nonempty
            sample_after_transfer_can_discharge
      · exact TraceAccepted.done sampleAfterDischarge

theorem sample_return_exit_accepted :
    ModeledExitStep sampleAfterDischarge ModeledExit.return sampleAfterReturn := by
  exact ModeledExitStep.step_return sampleAfterDischarge sample_after_discharge_has_no_unresolved

theorem sample_panic_exit_accepted :
    ModeledExitStep sampleAfterDischarge ModeledExit.panic sampleAfterPanic := by
  exact ModeledExitStep.step_panic sampleAfterDischarge sample_after_discharge_has_no_unresolved

theorem sample_opaque_exit_recorded :
    ModeledExitStep sampleAfterDischarge ModeledExit.opaqueBoundary sampleEnd := by
  exact
    ModeledExitStep.step_opaque
      sampleAfterDischarge
      delivery
      delivery_nonempty
      sample_after_panic_can_cross_opaque_boundary
      sampleTemplateAssumption
      template_assumption_is_current
      sampleTemplateRequirement
      sample_template_assumption_matches
      sampleLedger
      sample_opaque_ledger_recorded

end Kobo
