#[path = "harness/agreement.rs"]
mod agreement;
#[path = "harness/events.rs"]
mod events;
#[path = "harness/facade.rs"]
mod facade;
#[path = "harness/facade_manifest.rs"]
mod facade_manifest;
#[path = "harness/failures.rs"]
mod failures;
#[path = "harness/network_support.rs"]
mod network_support;
#[path = "harness/record_boundary.rs"]
mod record_boundary;
#[path = "harness/runner.rs"]
mod runner;
#[path = "harness/source.rs"]
mod source;
#[path = "harness/storage_support.rs"]
mod storage_support;
#[path = "harness/tokio_support.rs"]
mod tokio_support;

pub use agreement::check_harness_agreement;
use runner::run_generated_harness;
