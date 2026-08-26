// UI Models for wrkflw
use chrono::Local;
use std::path::PathBuf;
use std::sync::Arc;
use wrkflw_executor::{JobStatus, StepStatus};
use wrkflw_logging::symbols;
use wrkflw_parser::workflow::WorkflowDefinition;

/// Type alias for the complex execution result type
pub type ExecutionResultMsg = (usize, Result<(Vec<wrkflw_executor::JobResult>, ()), String>);

/// Result of trigger evaluation for TUI display
#[derive(Debug, Clone)]
pub enum TriggerMatchStatus {
    /// Workflow would trigger based on current diff
    Matched(String),
    /// Workflow would NOT trigger
    Skipped(String),
}

/// Represents an individual workflow file
pub struct Workflow {
    pub name: String,
    pub path: PathBuf,
    pub selected: bool,
    pub status: WorkflowStatus,
    pub execution_details: Option<WorkflowExecution>,
    pub job_names: Vec<String>,
    pub trigger_match: Option<TriggerMatchStatus>,
    /// Parsed workflow definition. Populated at load time so the Dashboard
    /// preview / mini-DAG don't have to reparse on every render. `None` when
    /// the file failed to parse (we still show the row so the user sees it).
    pub definition: Option<Arc<WorkflowDefinition>>,
}

impl Workflow {
    /// The execution entry for the job at `idx` in the jobs pane.
    ///
    /// The pane renders `job_names` (sorted at load time), while
    /// `execution_details.jobs` is in EXECUTION order and only contains jobs
    /// that have started — an index is not transferable between the two, so
    /// resolve by name. Matrix jobs execute as expanded combinations named
    /// `"<job> (<combo>)"`; when the template name has no exact entry, the
    /// first combination stands in.
    pub fn job_execution_at(&self, idx: usize) -> Option<&JobExecution> {
        let name = self.job_names.get(idx)?;
        let exec = self.execution_details.as_ref()?;
        exec.jobs.iter().find(|j| &j.name == name).or_else(|| {
            let prefix = format!("{} (", name);
            exec.jobs.iter().find(|j| j.name.starts_with(&prefix))
        })
    }

    /// Number of rows in the jobs pane. Falls back to the executed jobs when
    /// the definition failed to parse and `job_names` is empty.
    pub fn job_pane_len(&self) -> usize {
        if !self.job_names.is_empty() {
            self.job_names.len()
        } else {
            self.execution_details
                .as_ref()
                .map(|e| e.jobs.len())
                .unwrap_or(0)
        }
    }
}

/// A workflow queued for execution, with its own target job
pub struct QueuedExecution {
    pub workflow_idx: usize,
    pub target_job: Option<String>,
}

/// Status of a workflow
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowStatus {
    NotStarted,
    Running,
    Success,
    Failed,
    Skipped,
}

/// Detailed execution information
pub struct WorkflowExecution {
    pub jobs: Vec<JobExecution>,
    pub start_time: chrono::DateTime<Local>,
    pub end_time: Option<chrono::DateTime<Local>>,
    pub logs: Vec<String>,
    pub progress: f64, // 0.0 - 1.0 for progress bar
}

/// Job execution details
pub struct JobExecution {
    pub name: String,
    pub status: JobStatus,
    pub steps: Vec<StepExecution>,
    pub logs: Vec<String>,
    /// Wall-clock start/end from the executor. `None` for jobs that never
    /// ran (skipped) or synthetic results (validation, error placeholders).
    pub started_at: Option<std::time::SystemTime>,
    pub finished_at: Option<std::time::SystemTime>,
}

impl JobExecution {
    /// Started but not finished — a live progress row. While this is true
    /// the `status` field is a placeholder and must not be read.
    pub fn is_running(&self) -> bool {
        self.started_at.is_some() && self.finished_at.is_none()
    }
}

/// Step execution details
pub struct StepExecution {
    pub name: String,
    pub status: StepStatus,
    pub output: String,
    /// Wall-clock start/end from the executor. `None` for synthetic results.
    pub started_at: Option<std::time::SystemTime>,
    pub finished_at: Option<std::time::SystemTime>,
}

impl StepExecution {
    /// Started but not finished — a live progress row. While this is true
    /// the `status` field is a placeholder and must not be read.
    pub fn is_running(&self) -> bool {
        self.started_at.is_some() && self.finished_at.is_none()
    }
}

/// Severity level for status bar toast messages
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum StatusSeverity {
    Success,
    Info,
    Warning,
    #[default]
    Error,
}

/// Log filter levels
#[derive(Debug, Clone, PartialEq)]
pub enum LogFilterLevel {
    Info,
    Warning,
    Error,
    Success,
    Trigger,
    All,
}

impl LogFilterLevel {
    pub fn matches(&self, log: &str) -> bool {
        match self {
            LogFilterLevel::Info => {
                log.contains(symbols::INFO) || (log.contains("INFO") && !log.contains("SUCCESS"))
            }
            LogFilterLevel::Warning => log.contains(symbols::WARNING) || log.contains("WARN"),
            LogFilterLevel::Error => log.contains(symbols::FAILURE) || log.contains("ERROR"),
            LogFilterLevel::Success => {
                log.contains(symbols::SUCCESS) || log.contains("SUCCESS") || log.contains("success")
            }
            LogFilterLevel::Trigger => {
                log.contains("Triggering") || log.contains("triggered") || log.contains("TRIG")
            }
            LogFilterLevel::All => true,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            LogFilterLevel::All => LogFilterLevel::Info,
            LogFilterLevel::Info => LogFilterLevel::Warning,
            LogFilterLevel::Warning => LogFilterLevel::Error,
            LogFilterLevel::Error => LogFilterLevel::Success,
            LogFilterLevel::Success => LogFilterLevel::Trigger,
            LogFilterLevel::Trigger => LogFilterLevel::All,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            LogFilterLevel::All => "ALL",
            LogFilterLevel::Info => "INFO",
            LogFilterLevel::Warning => "WARNING",
            LogFilterLevel::Error => "ERROR",
            LogFilterLevel::Success => "SUCCESS",
            LogFilterLevel::Trigger => "TRIGGER",
        }
    }
}
