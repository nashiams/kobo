use crate::digest::stable_hash;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedHistory {
    pub id: String,
    pub scheduler: String,
    pub fault: String,
    pub cancellation: String,
    pub history_hash: String,
}

pub fn enumerate_bounded_histories(
    function: &str,
    count: u64,
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
) -> Vec<BoundedHistory> {
    if count == 0
        || scheduler_dimensions.is_empty()
        || fault_dimensions.is_empty()
        || cancellation_points.is_empty()
    {
        return Vec::new();
    }
    (0..count)
        .map(|index| {
            let scheduler = dimension_at(scheduler_dimensions, index);
            let fault = dimension_at(fault_dimensions, index / scheduler_dimensions.len() as u64);
            let cancellation = dimension_at(
                cancellation_points,
                index / (scheduler_dimensions.len() * fault_dimensions.len()) as u64,
            );
            let id = format!("history-{function}-{index}");
            let material = bounded_history_material(&id, scheduler, fault, cancellation);
            BoundedHistory {
                id,
                scheduler: scheduler.to_owned(),
                fault: fault.to_owned(),
                cancellation: cancellation.to_owned(),
                history_hash: stable_hash(&material),
            }
        })
        .collect()
}

pub fn bounded_history_material(
    id: &str,
    scheduler: &str,
    fault: &str,
    cancellation: &str,
) -> String {
    format!("{id}:{scheduler}:{fault}:{cancellation}")
}

fn dimension_at(dimensions: &[String], index: u64) -> &str {
    let selected = index as usize % dimensions.len();
    dimensions[selected].as_str()
}
