// Dynamic (expression) matrices: `strategy.matrix` given as a runtime
// expression such as `${{ fromJSON(needs.plan.outputs.matrix) }}`, the
// standard GitHub Actions pattern for resolver-computed fan-outs.
use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};
use wrkflw_matrix as matrix;

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

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

#[test]
fn matrix_enum_deserializes_mapping_as_config() {
    let m: matrix::Matrix = serde_yaml::from_str("os: [ubuntu, macos]\nnode: [18, 20]").unwrap();
    let config = m.as_config().expect("mapping should be Config");
    assert_eq!(config.parameters.len(), 2);
    assert!(m.as_expression().is_none());
}

#[test]
fn matrix_enum_deserializes_expression_as_string() {
    let m: matrix::Matrix =
        serde_yaml::from_str("${{ fromJSON(needs.plan.outputs.matrix) }}").unwrap();
    assert_eq!(
        m.as_expression(),
        Some("${{ fromJSON(needs.plan.outputs.matrix) }}")
    );
    assert!(m.as_config().is_none());
}

#[test]
fn config_from_json_parses_include_shape() {
    // The shape a resolver job typically emits.
    let json = r#"{"include": [{"service": "service-ying"}, {"service": "service-tao"}]}"#;
    let config = matrix::config_from_json(json).unwrap();
    let combos = matrix::expand_matrix(&config).unwrap();
    assert_eq!(combos.len(), 2);
    assert!(combos.iter().all(|c| c.values.contains_key("service")));
}

#[test]
fn config_from_json_rejects_non_objects() {
    assert!(matrix::config_from_json("[1, 2, 3]").is_err());
    assert!(matrix::config_from_json("just text").is_err());
}

#[tokio::test]
async fn dynamic_matrix_expands_from_needs_outputs() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    let workflow = r#"
name: CI
on: push
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.emit.outputs.matrix }}
    steps:
      - id: emit
        run: echo 'matrix={"include":[{"service":"alpha"},{"service":"beta"}]}' >> "$GITHUB_OUTPUT"
  fan:
    runs-on: ubuntu-latest
    needs: [plan]
    strategy:
      matrix: ${{ fromJSON(needs.plan.outputs.matrix) }}
    steps:
      - run: echo "service is ${{ matrix.service }}"
"#;
    write_file(&workflow_path, workflow);

    let result = execute_workflow(&workflow_path, emulation_config())
        .await
        .expect("workflow execution failed");

    let fan_jobs: Vec<&str> = result
        .jobs
        .iter()
        .map(|j| j.name.as_str())
        .filter(|n| n.starts_with("fan"))
        .collect();
    assert_eq!(
        fan_jobs.len(),
        2,
        "expected the dynamic matrix to expand to 2 combinations, got: {:?}",
        fan_jobs
    );
    assert!(
        fan_jobs.iter().any(|n| n.contains("alpha")) && fan_jobs.iter().any(|n| n.contains("beta")),
        "expected one combination per service, got: {:?}",
        fan_jobs
    );
}

#[tokio::test]
async fn dynamic_matrix_with_unresolvable_expression_fails_loudly() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    // `needs.missing` never exists — the resolved value cannot become a
    // matrix object and the job must fail rather than silently skip.
    let workflow = r#"
name: CI
on: push
jobs:
  fan:
    runs-on: ubuntu-latest
    strategy:
      matrix: ${{ fromJSON(needs.missing.outputs.matrix) }}
    steps:
      - run: echo "should not run"
"#;
    write_file(&workflow_path, workflow);

    let result = execute_workflow(&workflow_path, emulation_config()).await;
    match result {
        Err(_) => {}
        Ok(res) => {
            assert!(
                res.jobs.iter().all(|j| !format!("{:?}", j.status).contains("Success")),
                "an unresolvable matrix expression must not yield successful jobs"
            );
        }
    }
}
