//! Live execution progress — job/step start/finish events emitted while a
//! workflow runs, so a UI can draw real-time state instead of waiting for
//! the final `Vec<JobResult>`.
//!
//! The sink is process-global rather than threaded through the execution
//! contexts: `JobServices`/`*ExecutionContext` literals exist at a dozen
//! sites (tests included) and the executor runs one workflow at a time in
//! every current embedding (TUI queue, CLI). A consumer installs a sender
//! with [`set_progress_sink`] before starting a run and clears it after;
//! with no sink installed, emission is a no-op.
//!
//! Child jobs of reusable workflows emit too (under their own names) — the
//! final result replaces live state wholesale, so transient child rows are
//! informative during the run and reconciled at the end.

use crate::engine::{JobStatus, StepResult, StepStatus};
use std::sync::Mutex;
use std::time::SystemTime;

/// A live execution event. `at` carries the executor's wall-clock stamp so
/// consumers place events on the same time axis as the final results.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    JobStarted {
        job: String,
        at: SystemTime,
    },
    JobFinished {
        job: String,
        status: JobStatus,
        at: SystemTime,
    },
    StepStarted {
        job: String,
        step: String,
        at: SystemTime,
    },
    StepFinished {
        job: String,
        step: String,
        status: StepStatus,
        output: String,
        at: SystemTime,
    },
}

pub type ProgressSender = tokio::sync::mpsc::UnboundedSender<ProgressEvent>;

static SINK: Mutex<Option<ProgressSender>> = Mutex::new(None);

/// Install (or clear, with `None`) the process-wide progress sink.
pub fn set_progress_sink(sender: Option<ProgressSender>) {
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = sender;
}

pub(crate) fn emit(event: ProgressEvent) {
    if let Some(tx) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        // A closed receiver just means the consumer went away; never fail a run over it.
        let _ = tx.send(event);
    }
}

pub(crate) fn emit_job_started(job: &str, at: SystemTime) {
    emit(ProgressEvent::JobStarted {
        job: job.to_string(),
        at,
    });
}

pub(crate) fn emit_job_finished(job: &str, status: JobStatus) {
    emit(ProgressEvent::JobFinished {
        job: job.to_string(),
        status,
        at: SystemTime::now(),
    });
}

pub(crate) fn emit_step_started(job: &str, step: &str) {
    emit(ProgressEvent::StepStarted {
        job: job.to_string(),
        step: step.to_string(),
        at: SystemTime::now(),
    });
}

/// Emit a finish event from a stamped `StepResult` (the step loop calls this
/// right after the result lands in `StepLoopState`).
pub(crate) fn emit_step_finished(job: &str, result: &StepResult) {
    emit(ProgressEvent::StepFinished {
        job: job.to_string(),
        step: result.name.clone(),
        status: result.status,
        output: result.output.clone(),
        at: result.finished_at.unwrap_or_else(SystemTime::now),
    });
}
