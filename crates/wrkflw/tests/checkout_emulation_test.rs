// Emulated checkout semantics: same-repo checkouts copy the WORKFLOW'S
// project tree (not the invocation cwd), and org forks of actions/checkout
// (e.g. `MyOrg/checkout`) get the same treatment instead of falling through
// to remote resolution — which 404s for private forks and leaves the
// workspace empty.
use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};

fn emulation_config() -> ExecutionConfig {
    ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
    }
}

/// Build a fake project: root marker file + .github/workflows/ci.yml.
fn project_with_workflow(workflow: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let workflows = dir.path().join(".github").join("workflows");
    fs::create_dir_all(&workflows).unwrap();
    fs::write(dir.path().join("marker.json"), r#"{"project": "lab"}"#).unwrap();
    let path = workflows.join("ci.yml");
    fs::write(&path, workflow).unwrap();
    (dir, path)
}

#[tokio::test]
async fn fork_checkout_copies_the_workflow_project_tree() {
    // The fork name (SomeOrg/checkout) must be treated as a checkout, and
    // the copy source must be the project the workflow belongs to — the test
    // process cwd is the wrkflw source tree, which has no marker.json.
    let workflow = r#"
name: CI
on: push
jobs:
  resolve:
    runs-on: ubuntu-latest
    steps:
      - uses: SomeOrg/checkout@main
      - run: test -f marker.json
"#;
    let (_dir, path) = project_with_workflow(workflow);

    let result = execute_workflow(&path, emulation_config())
        .await
        .expect("workflow execution failed");

    assert!(
        result
            .jobs
            .iter()
            .all(|j| format!("{:?}", j.status).contains("Success")),
        "expected the fork checkout to populate the workspace: {:?}",
        result
            .jobs
            .iter()
            .map(|j| (&j.name, &j.status))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn checkout_path_input_lands_in_subdirectory() {
    let workflow = r#"
name: CI
on: push
jobs:
  resolve:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          path: nested/here
      - run: test -f nested/here/marker.json && test ! -f marker.json
"#;
    let (_dir, path) = project_with_workflow(workflow);

    let result = execute_workflow(&path, emulation_config())
        .await
        .expect("workflow execution failed");

    assert!(
        result
            .jobs
            .iter()
            .all(|j| format!("{:?}", j.status).contains("Success")),
        "expected checkout with `path:` to copy into the subdirectory"
    );
}

#[tokio::test]
async fn subpath_actions_named_checkout_are_not_intercepted() {
    // owner/repo/checkout sub-path actions must NOT be treated as checkout —
    // this one resolves remotely, fails, and falls back to the built-in
    // mapping (a no-op) — so the marker file must NOT appear.
    let workflow = r#"
name: CI
on: push
jobs:
  resolve:
    runs-on: ubuntu-latest
    steps:
      - uses: SomeOrg/tools/checkout@main
      - run: test ! -f marker.json
"#;
    let (_dir, path) = project_with_workflow(workflow);

    let result = execute_workflow(&path, emulation_config())
        .await
        .expect("workflow execution failed");

    assert!(
        result
            .jobs
            .iter()
            .all(|j| format!("{:?}", j.status).contains("Success")),
        "sub-path action must not populate the workspace as a checkout"
    );
}

#[tokio::test]
async fn checkout_brings_the_git_directory() {
    // Real actions/checkout produces a git repository — emulation must too,
    // or every diff-based affected computation dies on "not a git repository".
    let workflow = r#"
name: CI
on: push
jobs:
  resolve:
    runs-on: ubuntu-latest
    steps:
      - uses: SomeOrg/checkout@main
      - run: git rev-parse HEAD
"#;
    let (dir, path) = project_with_workflow(workflow);

    // Turn the fixture project into a real git repo with one commit.
    for args in [
        vec!["init", "-q"],
        vec!["-c", "user.email=lab@test", "-c", "user.name=lab", "add", "."],
        vec![
            "-c",
            "user.email=lab@test",
            "-c",
            "user.name=lab",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    let result = execute_workflow(&path, emulation_config())
        .await
        .expect("workflow execution failed");

    assert!(
        result
            .jobs
            .iter()
            .all(|j| format!("{:?}", j.status).contains("Success")),
        "expected git to work inside the emulated checkout"
    );
}
