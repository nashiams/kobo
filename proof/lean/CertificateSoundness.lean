import SampleTrace

namespace Kobo

structure SampleCertificateBridge where
  rustFixturePath : String
  rustRuleIds : List RuleId
  leanTraceAccepted : TraceAccepted sampleStart sampleTrace sampleEnd
  templateCurrent : templateAssumptionIsCurrent sampleTemplateAssumption
  opaqueLedger : sampleEvidence.opaqueLedgerRecorded = true

def sampleCertificateBridge : SampleCertificateBridge :=
  {
    rustFixturePath := "crates/compiler/kobo-proof/fixtures/v16_sample.kproof",
    rustRuleIds := sampleTrace,
    leanTraceAccepted := sample_trace_accepted,
    templateCurrent := template_assumption_is_current,
    opaqueLedger := sample_trace_has_opaque_ledger
  }

theorem sample_certificate_trace_sound :
    TraceAccepted sampleStart sampleTrace sampleEnd
      /\ templateAssumptionIsCurrent sampleTemplateAssumption
      /\ sampleEvidence.opaqueLedgerRecorded = true := by
  constructor
  · exact sampleCertificateBridge.leanTraceAccepted
  · constructor
    · exact sampleCertificateBridge.templateCurrent
    · exact sampleCertificateBridge.opaqueLedger

end Kobo
