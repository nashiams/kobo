use super::{ModelComparisonSpec, WardModelRun, WardModelStep, WardModelStepKind};

pub(crate) fn execute_ward_model(spec: &ModelComparisonSpec, seed: u64) -> WardModelRun {
    let mut interpreter = WardModelInterpreter::new(seed);
    for step in &spec.steps {
        interpreter.execute(step);
    }
    let (events, states, obligations, scheduler_preset) = interpreter.finish();
    WardModelRun {
        source: spec.source,
        engine: if spec.source == "ward_model" {
            "ward-model-interpreter"
        } else {
            "legacy-directive-interpreter"
        },
        seed,
        ir: spec.steps.clone(),
        events,
        states,
        obligations,
        scheduler_preset,
        steps_executed: spec.steps.len(),
    }
}

struct WardModelInterpreter {
    seed: u64,
    events: Vec<String>,
    states: Vec<(String, String)>,
    obligations: Vec<(String, String)>,
    scheduler_preset: Option<String>,
}

impl WardModelInterpreter {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            events: Vec::new(),
            states: Vec::new(),
            obligations: Vec::new(),
            scheduler_preset: None,
        }
    }

    fn execute(&mut self, step: &WardModelStep) {
        match &step.kind {
            WardModelStepKind::EmitEvent(event) => self.events.push(event.clone()),
            WardModelStepKind::SetObligation { binding, state } => {
                self.set_obligation(binding, state);
            }
            WardModelStepKind::SetState { name, value } => {
                self.set_state(name, value);
            }
            WardModelStepKind::SchedulerAssumption { preset } => {
                self.scheduler_preset = Some(preset.clone());
            }
            WardModelStepKind::Transition { boundary, .. } => {
                self.events.push(boundary.event_kind().to_owned());
            }
            WardModelStepKind::BoundaryCall(boundary) => {
                self.events.push(boundary.event_kind().to_owned());
            }
        }
    }

    fn set_state(&mut self, name: &str, value: &str) {
        if let Some((_, existing)) = self
            .states
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
        {
            *existing = value.to_owned();
            return;
        }
        self.states.push((name.to_owned(), value.to_owned()));
    }

    fn set_obligation(&mut self, binding: &str, state: &str) {
        if let Some((_, existing)) = self
            .obligations
            .iter_mut()
            .find(|(candidate, _)| candidate == binding)
        {
            *existing = state.to_owned();
            return;
        }
        self.obligations
            .push((binding.to_owned(), state.to_owned()));
    }

    fn finish(
        self,
    ) -> (
        Vec<String>,
        Vec<(String, String)>,
        Vec<(String, String)>,
        Option<String>,
    ) {
        let _ = self.seed;
        (
            self.events,
            self.states,
            self.obligations,
            self.scheduler_preset,
        )
    }
}

pub(crate) fn model_run_json(model_run: &WardModelRun) -> serde_json::Value {
    serde_json::json!({
        "source": model_run.source,
        "engine": model_run.engine,
        "semantics": "typed_ward_model_ir",
        "seed": model_run.seed,
        "ir": model_run
            .ir
            .iter()
            .map(|step| {
                let mut value = step.kind.to_json();
                if let Some(object) = value.as_object_mut() {
                    object.insert("span_start".to_owned(), serde_json::json!(step.span.0));
                    object.insert("span_end".to_owned(), serde_json::json!(step.span.1));
                }
                value
            })
            .collect::<Vec<_>>(),
        "events": model_run.events,
        "states": model_run
            .states
            .iter()
            .map(|(name, value)| {
                serde_json::json!({
                    "name": name,
                    "value": value,
                })
            })
            .collect::<Vec<_>>(),
        "obligations": model_run
            .obligations
            .iter()
            .map(|(binding, state)| {
                serde_json::json!({
                    "binding": binding,
                    "state": state,
                })
            })
            .collect::<Vec<_>>(),
        "scheduler_assumptions": {
            "preset": model_run.scheduler_preset,
            "seed": model_run.seed,
            "same_as_implementation": true,
        },
        "steps_executed": model_run.steps_executed,
    })
}
