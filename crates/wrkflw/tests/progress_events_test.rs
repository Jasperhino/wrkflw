// Live progress events: while a workflow executes, the engine emits
// job/step start/finish events through the process-global progress sink
// (crates/executor/src/progress.rs) so a UI can render real-time state.
//
// This file holds exactly one test — the sink is process-global, and a
// dedicated integration-test binary guarantees no concurrent test in the
// same process can interleave its own events.

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, JobStatus, RuntimeType};
use wrkflw_lib::executor::progress::{set_progress_sink, ProgressEvent};

#[tokio::test]
async fn progress_events_stream_job_and_step_lifecycle() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");
    fs::write(
        &workflow_path,
        r#"
name: progress
on: push
jobs:
  first:
    runs-on: ubuntu-latest
    steps:
      - name: greet
        id: greet
        run: |
          echo hello
          echo "who=world" >> "$GITHUB_OUTPUT"
          echo "GREETED=yes" >> "$GITHUB_ENV"
      - name: skipped step
        if: ${{ false }}
        run: echo never
  second:
    runs-on: ubuntu-latest
    needs: [first]
    steps:
      - name: follow
        run: echo after
"#,
    )
    .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    set_progress_sink(Some(tx));
    let result = execute_workflow(
        &workflow_path,
        ExecutionConfig {
            runtime_type: RuntimeType::Emulation,
            verbose: false,
            preserve_containers_on_failure: false,
            secrets_config: None,
            show_action_messages: false,
            target_job: None,
        },
    )
    .await
    .expect("workflow execution failed");
    set_progress_sink(None);

    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }

    // Compact trace like "JS:first SS:first/greet SF:first/greet ... JF:second"
    let trace: Vec<String> = events
        .iter()
        .map(|ev| match ev {
            ProgressEvent::JobStarted { job, .. } => format!("JS:{}", job),
            ProgressEvent::JobFinished { job, .. } => format!("JF:{}", job),
            ProgressEvent::StepStarted { job, step, .. } => format!("SS:{}/{}", job, step),
            ProgressEvent::StepFinished { job, step, .. } => format!("SF:{}/{}", job, step),
        })
        .collect();

    let pos = |needle: &str| -> usize {
        trace
            .iter()
            .position(|t| t == needle)
            .unwrap_or_else(|| panic!("missing '{}' in trace: {:?}", needle, trace))
    };

    // Lifecycle ordering within and across jobs.
    assert!(pos("JS:first") < pos("SS:first/greet"));
    assert!(pos("SS:first/greet") < pos("SF:first/greet"));
    assert!(pos("SF:first/greet") < pos("JF:first"));
    assert!(pos("JF:first") < pos("JS:second"), "trace: {:?}", trace);
    assert!(pos("SS:second/follow") < pos("SF:second/follow"));
    assert!(pos("SF:second/follow") < pos("JF:second"));

    // Condition-skipped steps still report both edges.
    assert!(pos("SS:first/skipped step") < pos("SF:first/skipped step"));

    // Finish events carry final statuses and timestamps consistent with
    // the results (which all succeeded here).
    for ev in &events {
        if let ProgressEvent::JobFinished { status, .. } = ev {
            assert_eq!(*status, JobStatus::Success);
        }
    }
    assert!(result.jobs.iter().all(|j| j.status == JobStatus::Success));

    // Event timestamps agree with the stamped results: the finish event of
    // "greet" matches the step's finished_at.
    let greet_finish_at = events
        .iter()
        .find_map(|ev| match ev {
            ProgressEvent::StepFinished { job, step, at, .. }
                if job == "first" && step == "greet" =>
            {
                Some(*at)
            }
            _ => None,
        })
        .expect("no finish event for greet");
    let greet_result = result
        .jobs
        .iter()
        .find(|j| j.name == "first")
        .and_then(|j| j.steps.iter().find(|s| s.name == "greet"))
        .expect("greet result missing");
    assert_eq!(greet_result.finished_at, Some(greet_finish_at));

    // Per-step captures for the inspector panes: the env snapshot exists
    // and masks the ambient token; $GITHUB_OUTPUT / $GITHUB_ENV writes are
    // recorded on the step that made them.
    assert!(
        !greet_result.env.is_empty(),
        "expected an env snapshot on the step result"
    );
    if let Some((_, v)) = greet_result.env.iter().find(|(k, _)| k == "GITHUB_TOKEN") {
        assert!(v.is_empty() || v == "***", "GITHUB_TOKEN not masked: {v}");
    }
    assert_eq!(
        greet_result.outputs,
        vec![("who".to_string(), "world".to_string())]
    );
    assert_eq!(
        greet_result.env_writes,
        vec![("GREETED".to_string(), "yes".to_string())]
    );

    // The env written by "greet" is visible to the NEXT step's snapshot.
    let skipped_step = result
        .jobs
        .iter()
        .find(|j| j.name == "first")
        .and_then(|j| j.steps.iter().find(|s| s.name == "skipped step"))
        .expect("skipped step missing");
    assert!(
        skipped_step
            .env
            .iter()
            .any(|(k, v)| k == "GREETED" && v == "yes"),
        "GITHUB_ENV write not visible in the next step's env snapshot"
    );
}
