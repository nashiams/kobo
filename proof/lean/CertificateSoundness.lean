import SampleTrace

namespace Kobo

def sampleAccepted : Prop :=
  sampleTrace = [
    RuleId.create,
    RuleId.split,
    RuleId.transfer,
    RuleId.discharge,
    RuleId.return,
    RuleId.cancel,
    RuleId.panic,
    RuleId.opaque
  ] ∧ sampleEvidence.opaqueLedgerRecorded = true

theorem sample_certificate_trace_sound : sampleAccepted := by
  constructor <;> rfl

end Kobo
