use crate::digest::stable_hash;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedHistory {
    pub id: String,
    pub scheduler: String,
    pub fault: String,
    pub cancellation: String,
    pub history_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedExploration {
    pub histories: Vec<BoundedHistory>,
    pub expected_history_count: u64,
    pub is_complete: bool,
}

pub fn explore_bounded_histories(
    function: &str,
    history_bound: u64,
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
) -> BoundedExploration {
    let expected_history_count =
        expected_history_count(scheduler_dimensions, fault_dimensions, cancellation_points);
    if history_bound == 0 || expected_history_count == 0 {
        return BoundedExploration {
            histories: Vec::new(),
            expected_history_count,
            is_complete: false,
        };
    }
    let explored_count = history_bound.min(expected_history_count);
    BoundedExploration {
        histories: enumerate_bounded_histories(
            function,
            explored_count,
            scheduler_dimensions,
            fault_dimensions,
            cancellation_points,
        ),
        expected_history_count,
        is_complete: history_bound >= expected_history_count,
    }
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

fn expected_history_count(
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
) -> u64 {
    if scheduler_dimensions.is_empty()
        || fault_dimensions.is_empty()
        || cancellation_points.is_empty()
    {
        return 0;
    }
    (scheduler_dimensions.len() as u64)
        .saturating_mul(fault_dimensions.len() as u64)
        .saturating_mul(cancellation_points.len() as u64)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::explore_bounded_histories;

    fn dimensions(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn bounded_exploration_uses_dimension_product() {
        let scheduler_dimensions = dimensions(&["fifo", "round_robin"]);
        let fault_dimensions = dimensions(&["none", "timeout"]);
        let cancellation_points = dimensions(&["none", "await_recv"]);
        let exploration = explore_bounded_histories(
            "bounded_case",
            8,
            &scheduler_dimensions,
            &fault_dimensions,
            &cancellation_points,
        );

        let combinations = exploration
            .histories
            .iter()
            .map(|history| {
                format!(
                    "{}:{}:{}",
                    history.scheduler, history.fault, history.cancellation
                )
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(exploration.expected_history_count, 8);
        assert!(exploration.is_complete);
        assert_eq!(combinations.len(), 8);
    }

    #[test]
    fn bounded_exploration_marks_truncated_state_space_incomplete() {
        let scheduler_dimensions = dimensions(&["fifo", "round_robin"]);
        let fault_dimensions = dimensions(&["none", "timeout"]);
        let cancellation_points = dimensions(&["none"]);
        let exploration = explore_bounded_histories(
            "bounded_case",
            2,
            &scheduler_dimensions,
            &fault_dimensions,
            &cancellation_points,
        );

        assert_eq!(exploration.expected_history_count, 4);
        assert_eq!(exploration.histories.len(), 2);
        assert!(!exploration.is_complete);
    }
}
