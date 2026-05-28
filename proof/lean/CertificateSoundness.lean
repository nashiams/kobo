import SampleTrace

namespace Kobo

structure SampleCertificateBridge where
  rustFixturePath : String
  rustRuleIds : List RuleId
  leanTraceAccepted : TraceAccepted sampleStart sampleObligationTrace sampleAfterDischarge
  returnExitAccepted : ModeledExitStep sampleAfterDischarge ModeledExit.return sampleAfterReturn
  panicExitAccepted : ModeledExitStep sampleAfterDischarge ModeledExit.panic sampleAfterPanic
  opaqueExitRecorded : ModeledExitStep sampleAfterDischarge ModeledExit.opaqueBoundary sampleEnd
  templateCurrent : templateAssumptionIsCurrent sampleTemplateAssumption
  templateMatches : templateAssumptionMatches sampleTemplateAssumption sampleTemplateRequirement
  opaqueLedger : sampleEvidence.opaqueLedgerRecorded = true
  opaqueLedgerRecorded : opaqueLedgerRecorded sampleLedger delivery "external.queue"

def sampleCertificateBridge : SampleCertificateBridge :=
  {
    rustFixturePath := "crates/compiler/kobo-proof/fixtures/sample_trace.kproof",
    rustRuleIds := sampleTrace,
    leanTraceAccepted := sample_obligation_trace_accepted,
    returnExitAccepted := sample_return_exit_accepted,
    panicExitAccepted := sample_panic_exit_accepted,
    opaqueExitRecorded := sample_opaque_exit_recorded,
    templateCurrent := template_assumption_is_current,
    templateMatches := sample_template_assumption_matches,
    opaqueLedger := sample_trace_has_opaque_ledger,
    opaqueLedgerRecorded := sample_opaque_ledger_recorded
  }

theorem sample_certificate_trace_sound :
    TraceAccepted sampleStart sampleObligationTrace sampleAfterDischarge
      /\ ModeledExitStep sampleAfterDischarge ModeledExit.return sampleAfterReturn
      /\ ModeledExitStep sampleAfterDischarge ModeledExit.panic sampleAfterPanic
      /\ ModeledExitStep sampleAfterDischarge ModeledExit.opaqueBoundary sampleEnd
      /\ templateAssumptionIsCurrent sampleTemplateAssumption
      /\ templateAssumptionMatches sampleTemplateAssumption sampleTemplateRequirement
      /\ opaqueLedgerRecorded sampleLedger delivery "external.queue"
      /\ sampleEvidence.opaqueLedgerRecorded = true := by
  constructor
  · exact sampleCertificateBridge.leanTraceAccepted
  · constructor
    · exact sampleCertificateBridge.returnExitAccepted
    · constructor
      · exact sampleCertificateBridge.panicExitAccepted
      · constructor
        · exact sampleCertificateBridge.opaqueExitRecorded
        · constructor
          · exact sampleCertificateBridge.templateCurrent
          · constructor
            · exact sampleCertificateBridge.templateMatches
            · constructor
              · exact sampleCertificateBridge.opaqueLedgerRecorded
              · exact sampleCertificateBridge.opaqueLedger

end Kobo
