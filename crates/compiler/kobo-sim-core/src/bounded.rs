use crate::digest::stable_hash;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedHistory {
    pub id: String,
    pub scheduler: String,
    pub fault: String,
    pub cancellation: String,
    pub queue_capacity: u64,
    pub message_count: u64,
    pub retry_attempts: u64,
    pub timeout_path: u64,
    pub external_boundary_recording: u64,
    pub history_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedExploration {
    pub histories: Vec<BoundedHistory>,
    pub expected_history_count: u64,
    pub is_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedStateDimensions {
    pub queue_capacities: Vec<u64>,
    pub message_counts: Vec<u64>,
    pub retry_attempts: Vec<u64>,
    pub timeout_paths: Vec<u64>,
    pub external_boundary_recordings: Vec<u64>,
}

impl BoundedStateDimensions {
    pub fn from_bounds(
        queue_capacity: Option<u64>,
        message_count: Option<u64>,
        retry_attempts: Option<u64>,
        timeout_paths: Option<u64>,
        external_boundary_recordings: Option<u64>,
    ) -> Self {
        Self {
            queue_capacities: numeric_axis(queue_capacity),
            message_counts: numeric_axis(message_count),
            retry_attempts: numeric_axis(retry_attempts),
            timeout_paths: numeric_axis(timeout_paths),
            external_boundary_recordings: numeric_axis(external_boundary_recordings),
        }
    }

    fn is_empty(&self) -> bool {
        self.queue_capacities.is_empty()
            || self.message_counts.is_empty()
            || self.retry_attempts.is_empty()
            || self.timeout_paths.is_empty()
            || self.external_boundary_recordings.is_empty()
    }

    fn state_product(&self) -> u64 {
        self.queue_capacities
            .len()
            .saturating_mul(self.message_counts.len())
            .saturating_mul(self.retry_attempts.len())
            .saturating_mul(self.timeout_paths.len())
            .saturating_mul(self.external_boundary_recordings.len()) as u64
    }
}

pub fn explore_bounded_histories(
    function: &str,
    history_bound: u64,
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
    state_dimensions: &BoundedStateDimensions,
) -> BoundedExploration {
    let expected_history_count = expected_history_count(
        scheduler_dimensions,
        fault_dimensions,
        cancellation_points,
        state_dimensions,
    );
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
            state_dimensions,
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
    state_dimensions: &BoundedStateDimensions,
) -> Vec<BoundedHistory> {
    if count == 0
        || scheduler_dimensions.is_empty()
        || fault_dimensions.is_empty()
        || cancellation_points.is_empty()
        || state_dimensions.is_empty()
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
            let cancellation_stride = dimension_product(&[
                scheduler_dimensions.len(),
                fault_dimensions.len(),
                cancellation_points.len(),
            ]);
            let queue_capacity = numeric_dimension_at(
                &state_dimensions.queue_capacities,
                index / cancellation_stride,
            );
            let message_stride =
                cancellation_stride.saturating_mul(state_dimensions.queue_capacities.len() as u64);
            let message_count =
                numeric_dimension_at(&state_dimensions.message_counts, index / message_stride);
            let retry_stride =
                message_stride.saturating_mul(state_dimensions.message_counts.len() as u64);
            let retry_attempts =
                numeric_dimension_at(&state_dimensions.retry_attempts, index / retry_stride);
            let timeout_stride =
                retry_stride.saturating_mul(state_dimensions.retry_attempts.len() as u64);
            let timeout_path =
                numeric_dimension_at(&state_dimensions.timeout_paths, index / timeout_stride);
            let external_stride =
                timeout_stride.saturating_mul(state_dimensions.timeout_paths.len() as u64);
            let external_boundary_recording = numeric_dimension_at(
                &state_dimensions.external_boundary_recordings,
                index / external_stride,
            );
            let id = format!("history-{function}-{index}");
            let material = bounded_history_material(
                &id,
                scheduler,
                fault,
                cancellation,
                queue_capacity,
                message_count,
                retry_attempts,
                timeout_path,
                external_boundary_recording,
            );
            BoundedHistory {
                id,
                scheduler: scheduler.to_owned(),
                fault: fault.to_owned(),
                cancellation: cancellation.to_owned(),
                queue_capacity,
                message_count,
                retry_attempts,
                timeout_path,
                external_boundary_recording,
                history_hash: stable_hash(&material),
            }
        })
        .collect()
}

fn expected_history_count(
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
    state_dimensions: &BoundedStateDimensions,
) -> u64 {
    if scheduler_dimensions.is_empty()
        || fault_dimensions.is_empty()
        || cancellation_points.is_empty()
        || state_dimensions.is_empty()
    {
        return 0;
    }
    (scheduler_dimensions.len() as u64)
        .saturating_mul(fault_dimensions.len() as u64)
        .saturating_mul(cancellation_points.len() as u64)
        .saturating_mul(state_dimensions.state_product())
}

pub fn bounded_history_material(
    id: &str,
    scheduler: &str,
    fault: &str,
    cancellation: &str,
    queue_capacity: u64,
    message_count: u64,
    retry_attempts: u64,
    timeout_path: u64,
    external_boundary_recording: u64,
) -> String {
    format!(
        "{id}:{scheduler}:{fault}:{cancellation}:{queue_capacity}:{message_count}:{retry_attempts}:{timeout_path}:{external_boundary_recording}"
    )
}

fn dimension_at(dimensions: &[String], index: u64) -> &str {
    let selected = index as usize % dimensions.len();
    dimensions[selected].as_str()
}

fn numeric_axis(bound: Option<u64>) -> Vec<u64> {
    match bound {
        Some(0) => vec![0],
        Some(bound) => (1..=bound).collect(),
        None => Vec::new(),
    }
}

fn numeric_dimension_at(dimensions: &[u64], index: u64) -> u64 {
    let selected = index as usize % dimensions.len();
    dimensions[selected]
}

fn dimension_product(dimensions: &[usize]) -> u64 {
    dimensions.iter().fold(1_u64, |product, dimension| {
        product.saturating_mul(*dimension as u64)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{explore_bounded_histories, BoundedStateDimensions};

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
            &BoundedStateDimensions::from_bounds(Some(1), Some(1), Some(1), Some(1), Some(0)),
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
            &BoundedStateDimensions::from_bounds(Some(1), Some(1), Some(1), Some(1), Some(0)),
        );

        assert_eq!(exploration.expected_history_count, 4);
        assert_eq!(exploration.histories.len(), 2);
        assert!(!exploration.is_complete);
    }

    #[test]
    fn bounded_exploration_includes_state_dimension_product() {
        let scheduler_dimensions = dimensions(&["fifo", "round_robin"]);
        let fault_dimensions = dimensions(&["none"]);
        let cancellation_points = dimensions(&["none"]);
        let state_dimensions =
            BoundedStateDimensions::from_bounds(Some(2), Some(2), Some(2), Some(2), Some(1));
        let exploration = explore_bounded_histories(
            "bounded_case",
            32,
            &scheduler_dimensions,
            &fault_dimensions,
            &cancellation_points,
            &state_dimensions,
        );

        let combinations = exploration
            .histories
            .iter()
            .map(|history| {
                format!(
                    "{}:{}:{}:{}:{}",
                    history.queue_capacity,
                    history.message_count,
                    history.retry_attempts,
                    history.timeout_path,
                    history.external_boundary_recording
                )
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(exploration.expected_history_count, 32);
        assert!(exploration.is_complete);
        assert_eq!(combinations.len(), 16);
    }
}
