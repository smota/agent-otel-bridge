//! Development-only, offline smoke laboratory for fleet trace assertions.
//!
//! Default runs execute local fixtures with reproducible inputs. Explicit live
//! mode invokes bounded harness adapters; process success alone is inconclusive.

pub mod adapters;
pub mod fixtures;
pub mod live_evidence;
pub mod operations;
pub mod plan;
pub mod report;
pub mod runner;
pub mod telemetry;
pub mod telemetry_validation;
pub mod verifier;
pub mod visibility;

pub use plan::{FleetPlan, PlanScenario};
pub use runner::{RunArtifact, RunMode, Runner};
pub use verifier::{VerificationReport, Verifier};
