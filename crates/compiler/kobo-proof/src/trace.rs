use crate::{CoreTraceEvent, GeneratedTraceEvent};

pub(crate) fn sorted_core_trace(events: &[CoreTraceEvent]) -> Vec<&CoreTraceEvent> {
    let mut sorted = events.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.order.cmp(&right.order).then(left.id.cmp(&right.id)));
    sorted
}

pub(crate) fn sorted_generated_trace(events: &[GeneratedTraceEvent]) -> Vec<&GeneratedTraceEvent> {
    let mut sorted = events.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.order.cmp(&right.order).then(left.id.cmp(&right.id)));
    sorted
}

pub(crate) fn optional_text(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}
