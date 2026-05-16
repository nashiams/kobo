use std::collections::VecDeque;

use kobo_errors::KErrorCode;

use crate::core::{ModeledBoundary, ScenarioEvent, ScenarioFailure, ScenarioOptions};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskLifecycle {
    Runnable,
    Waiting,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SchedulerTask {
    id: u64,
    label: String,
    lifecycle: TaskLifecycle,
    poll_count: u64,
}

#[derive(Clone, Debug)]
struct SchedulerModel<'a> {
    options: &'a ScenarioOptions,
    next_task_id: u64,
    runnable_queue: VecDeque<u64>,
    tasks: Vec<SchedulerTask>,
    events: Vec<ScenarioEvent>,
}

pub(crate) fn modeled_boundary_events(
    boundary: &ModeledBoundary,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    let mut events = vec![deterministic_boundary_event(boundary, options.seed)];
    if matches!(boundary, ModeledBoundary::WardTask) {
        let mut scheduler = SchedulerModel::new(options);
        scheduler.record_task_boundary();
        events.extend(scheduler.into_events());
    }
    events
}

pub(crate) fn select_events(branch_count: u32, options: &ScenarioOptions) -> Vec<ScenarioEvent> {
    let mut scheduler = SchedulerModel::new(options);
    scheduler.record_select(branch_count.max(1));
    scheduler.into_events()
}

pub(crate) fn schedule_failure(
    boundary: &ModeledBoundary,
    options: &ScenarioOptions,
    active_obligation: Option<(&str, (usize, usize))>,
    span: (usize, usize),
) -> Option<ScenarioFailure> {
    if !matches!(options.sim_profile.as_str(), "deep" | "exhaustive")
        || !matches!(boundary, ModeledBoundary::WardTask)
    {
        return None;
    }
    let (binding, obligation_span) = active_obligation?;
    Some(ScenarioFailure {
        code: KErrorCode::K0100,
        message: format!(
            "scheduler deep interleaving exposed obligation `{binding}` across task boundary"
        ),
        primary_start: span.0,
        primary_end: span.1,
        events: vec![ScenarioEvent {
            kind: "scheduler-interleaving-leak".to_owned(),
            label: Some(binding.to_owned()),
            value: Some(obligation_span.0 as u64),
        }],
    })
}

pub(crate) fn portfolio_events(options: &ScenarioOptions) -> Vec<ScenarioEvent> {
    let budget = scheduler_budget(options);
    let mut events = vec![ScenarioEvent {
        kind: "scheduler-portfolio".to_owned(),
        label: Some(options.sim_profile.clone()),
        value: Some(budget),
    }];
    if options.sim_profile == "deep" {
        events.push(ScenarioEvent {
            kind: "scheduler-pct-seed".to_owned(),
            label: Some("deep".to_owned()),
            value: Some(options.seed.rotate_left(7)),
        });
    }
    if options.sim_profile == "exhaustive" {
        events.push(ScenarioEvent {
            kind: "scheduler-exhaustive-cap".to_owned(),
            label: Some("tiny-ward".to_owned()),
            value: Some(budget),
        });
    }
    events
}

pub(crate) fn scheduler_budget(options: &ScenarioOptions) -> u64 {
    options
        .event_budget
        .unwrap_or(match options.sim_profile.as_str() {
            "quick" => 64,
            "deep" => 1024,
            "replay" => 64,
            "exhaustive" => 16,
            _ => 64,
        })
}

impl<'a> SchedulerModel<'a> {
    fn new(options: &'a ScenarioOptions) -> Self {
        Self {
            options,
            next_task_id: 1,
            runnable_queue: VecDeque::new(),
            tasks: Vec::new(),
            events: Vec::new(),
        }
    }

    fn record_task_boundary(&mut self) {
        let task_id = self.enqueue_task("ward.task");
        self.wake_task(task_id);
        self.poll_task(task_id);
        if self.has_cancel_injection() {
            self.cancel_task(task_id);
        } else {
            self.complete_task(task_id);
        }
        if self.options.sim_profile == "deep" {
            self.record_pct_choice();
        }
    }

    fn record_select(&mut self, branch_count: u32) {
        let selected_branch = self.selected_branch(branch_count);
        self.events.push(ScenarioEvent {
            kind: "scheduler-select-start".to_owned(),
            label: Some(self.options.sim_profile.clone()),
            value: Some(branch_count as u64),
        });
        for branch_index in 0..branch_count {
            self.record_select_branch(branch_index, selected_branch);
        }
    }

    fn enqueue_task(&mut self, label: &str) -> u64 {
        let task_id = self.next_task_id;
        self.next_task_id += 1;
        self.tasks.push(SchedulerTask {
            id: task_id,
            label: label.to_owned(),
            lifecycle: TaskLifecycle::Runnable,
            poll_count: 0,
        });
        self.runnable_queue.push_back(task_id);
        self.events
            .push(task_event("scheduler-task-enqueued", label, task_id));
        self.record_queue_depth();
        task_id
    }

    fn wake_task(&mut self, task_id: u64) {
        if !self.runnable_queue.iter().any(|queued| *queued == task_id) {
            self.runnable_queue.push_back(task_id);
        }
        if let Some(label) = self.update_task(task_id, TaskLifecycle::Runnable) {
            self.events
                .push(task_event("scheduler-task-wakeup", &label, task_id));
        }
        self.record_queue_depth();
    }

    fn poll_task(&mut self, task_id: u64) {
        self.runnable_queue.retain(|queued| *queued != task_id);
        if let Some((label, poll_count)) = self.poll_task_state(task_id) {
            self.events
                .push(task_event("scheduler-task-polled", &label, poll_count));
        }
        self.record_queue_depth();
    }

    fn complete_task(&mut self, task_id: u64) {
        if let Some(label) = self.update_task(task_id, TaskLifecycle::Completed) {
            self.events
                .push(task_event("scheduler-task-completed", &label, task_id));
        }
    }

    fn cancel_task(&mut self, task_id: u64) {
        self.runnable_queue.retain(|queued| *queued != task_id);
        if let Some(label) = self.update_task(task_id, TaskLifecycle::Cancelled) {
            self.events
                .push(task_event("scheduler-cancel-path", &label, task_id));
            self.events
                .push(task_event("scheduler-future-dropped", &label, task_id));
        }
        self.record_queue_depth();
    }

    fn record_select_branch(&mut self, branch_index: u32, selected_branch: u32) {
        let label = format!("branch-{branch_index}");
        self.events.push(ScenarioEvent {
            kind: "scheduler-select-branch".to_owned(),
            label: Some(label.clone()),
            value: Some(branch_index as u64),
        });
        if branch_index == selected_branch {
            self.events.push(ScenarioEvent {
                kind: "scheduler-select-selected-branch".to_owned(),
                label: Some(label),
                value: Some(self.options.seed),
            });
            return;
        }
        self.events.push(ScenarioEvent {
            kind: "scheduler-select-cancelled-branch".to_owned(),
            label: Some(label.clone()),
            value: Some(selected_branch as u64),
        });
        self.events.push(ScenarioEvent {
            kind: "scheduler-future-dropped".to_owned(),
            label: Some(label),
            value: Some(branch_index as u64),
        });
    }

    fn record_pct_choice(&mut self) {
        self.events.push(ScenarioEvent {
            kind: "scheduler-choice".to_owned(),
            label: Some("deep:pct-preempt".to_owned()),
            value: Some(self.options.seed.rotate_left(7)),
        });
    }

    fn record_queue_depth(&mut self) {
        self.events.push(ScenarioEvent {
            kind: "scheduler-runnable-queue".to_owned(),
            label: Some("len".to_owned()),
            value: Some(self.runnable_queue.len() as u64),
        });
    }

    fn selected_branch(&self, branch_count: u32) -> u32 {
        (self.options.seed % u64::from(branch_count)) as u32
    }

    fn has_cancel_injection(&self) -> bool {
        self.options
            .inject
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .any(|hook| hook == "cancel")
    }

    fn task_mut(&mut self, task_id: u64) -> Option<&mut SchedulerTask> {
        self.tasks.iter_mut().find(|task| task.id == task_id)
    }

    fn update_task(&mut self, task_id: u64, lifecycle: TaskLifecycle) -> Option<String> {
        let task = self.task_mut(task_id)?;
        task.lifecycle = lifecycle;
        Some(task.label.clone())
    }

    fn poll_task_state(&mut self, task_id: u64) -> Option<(String, u64)> {
        let task = self.task_mut(task_id)?;
        task.lifecycle = TaskLifecycle::Waiting;
        task.poll_count += 1;
        Some((task.label.clone(), task.poll_count))
    }

    fn into_events(self) -> Vec<ScenarioEvent> {
        self.events
    }
}

fn deterministic_boundary_event(boundary: &ModeledBoundary, seed: u64) -> ScenarioEvent {
    match boundary {
        ModeledBoundary::WardTime => ScenarioEvent {
            kind: "deterministic-time".to_owned(),
            label: None,
            value: Some(seed.wrapping_mul(1_000).wrapping_add(17)),
        },
        ModeledBoundary::WardRandom => ScenarioEvent {
            kind: "deterministic-random".to_owned(),
            label: None,
            value: Some(seed.rotate_left(13) ^ 0x9e37_79b9_7f4a_7c15_u64),
        },
        ModeledBoundary::WardTask => ScenarioEvent {
            kind: "deterministic-task".to_owned(),
            label: Some("ward.task".to_owned()),
            value: Some(seed),
        },
        ModeledBoundary::WardStorage => ScenarioEvent {
            kind: "storage-boundary".to_owned(),
            label: Some("ward.storage".to_owned()),
            value: Some(seed),
        },
        ModeledBoundary::WardNetwork => ScenarioEvent {
            kind: "network-boundary".to_owned(),
            label: Some("ward.network".to_owned()),
            value: Some(seed),
        },
    }
}

fn task_event(kind: &str, label: &str, value: u64) -> ScenarioEvent {
    ScenarioEvent {
        kind: kind.to_owned(),
        label: Some(label.to_owned()),
        value: Some(value),
    }
}
