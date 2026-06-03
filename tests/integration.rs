// tests/integration.rs
use std::process::Command;
use std::path::Path;

fn thruline() -> Command {
    Command::new(env!("CARGO_BIN_EXE_thruline"))
}

fn write_tl(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

const BASIC_TL: &str = r#"
runner r {
  model: claude-sonnet-4-6
  system: "You are a test agent."
}
stage a {
  out: verdict as value
  runner: r
}
stage b {
  runner: r
}
thruline p {
  start: a
  routes {
    a.verdict == "ok" -> b
  }
}
"#;

#[test]
fn test_validate_valid_file() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline().args(["validate", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}

#[test]
fn test_validate_unknown_runner_fails() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "bad.line", r#"
stage a {
  runner: ghost
}
thruline p {
  start: a
  routes {}
}
"#);

    let out = thruline().args(["validate", tl.to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("ghost"), "expected 'ghost' in error: {}", stderr);
}

#[test]
fn test_validate_missing_file_errors() {
    let out = thruline().args(["validate", "/nonexistent/path.line"]).output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn test_inspect_shows_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline().args(["inspect", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("thruline: p"), "got: {}", stdout);
    assert!(stdout.contains("start: a"), "got: {}", stdout);
    assert!(stdout.contains("a.verdict"), "got: {}", stdout);
    assert!(stdout.contains("-> b"), "got: {}", stdout);
}

#[test]
fn test_inspect_shows_stages() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline().args(["inspect", tl.to_str().unwrap()]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stage: a"), "got: {}", stdout);
    assert!(stdout.contains("stage: b"), "got: {}", stdout);
}

#[test]
fn test_run_stdio_emits_pipeline_start_and_stage_invoke() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should emit pipeline_start and stage_invoke as NDJSON lines
    assert!(stdout.contains(r#""event":"pipeline_start""#), "got: {}", stdout);
    assert!(stdout.contains(r#""protocol":"1""#), "got: {}", stdout);
    assert!(stdout.contains(r#""event":"stage_invoke""#), "got: {}", stdout);
    assert!(stdout.contains(r#""stage":"a""#), "got: {}", stdout);
    // Runner spec should be embedded in stage_invoke
    assert!(stdout.contains(r#""model":"claude-sonnet-4-6""#), "got: {}", stdout);
}

#[test]
fn test_run_creates_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Extract run_id from the pipeline_start event
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first_line = stdout.lines().next().unwrap();
    let event: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let run_id = event["run_id"].as_str().unwrap().to_string();

    // State file should exist
    let home = std::env::var("HOME").unwrap();
    let state_path = std::path::PathBuf::from(&home)
        .join(".thruline/runs")
        .join(&run_id)
        .join("state.json");
    assert!(state_path.exists(), "state file not found at {}", state_path.display());
}

#[test]
fn test_runs_command_shows_created_run() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    // Create a run
    let run_out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output()
        .unwrap();
    assert!(run_out.status.success());
    let stdout = String::from_utf8_lossy(&run_out.stdout);
    let first_line = stdout.lines().next().unwrap();
    let event: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let run_id = event["run_id"].as_str().unwrap().to_string();

    // List runs — should show our run
    let list_out = thruline().args(["runs"]).output().unwrap();
    assert!(list_out.status.success());
    let list_stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        list_stdout.contains(&run_id) || list_stdout.contains("p"),
        "run not found in list: {}",
        list_stdout
    );
}

#[test]
fn test_validate_with_import() {
    let dir = tempfile::tempdir().unwrap();

    // Write runners to a separate file
    write_tl(dir.path(), "runners.line", r#"
runner r {
  model: claude-sonnet-4-6
  system: "runner"
}
"#);

    // Main file imports runners.line
    let tl = write_tl(dir.path(), "main.line", r#"
import "runners.line"
stage a { runner: r }
thruline p {
  start: a
  routes {}
}
"#);

    let out = thruline().args(["validate", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}

#[test]
fn test_pipeline_done_includes_value_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let run_out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let run_id = serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(&run_out.stdout).lines().next().unwrap()
    ).unwrap()["run_id"].as_str().unwrap().to_string();

    // a routes to b on verdict==ok; b has no routes → pipeline_done
    thruline().args(["resume", &run_id, "--stage", "a", "--artifact", "verdict=ok"])
        .output().unwrap();
    let done_out = thruline()
        .args(["resume", &run_id, "--stage", "b"])
        .output().unwrap();
    let done_stdout = String::from_utf8_lossy(&done_out.stdout);

    let done: serde_json::Value = done_stdout.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .find(|v: &serde_json::Value| v["event"] == "pipeline_done")
        .expect("no pipeline_done");

    let outputs = &done["outputs"];
    assert!(outputs.is_object(), "outputs should be an object, got: {}", outputs);
    assert_eq!(outputs["a.verdict"], "ok", "a.verdict missing: {}", outputs);
}

#[test]
fn test_resume_emits_stage_complete() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let run_out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let run_id = serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(&run_out.stdout).lines().next().unwrap()
    ).unwrap()["run_id"].as_str().unwrap().to_string();

    let resume_out = thruline()
        .args(["resume", &run_id, "--stage", "a", "--artifact", "verdict=ok"])
        .output().unwrap();
    let resume_stdout = String::from_utf8_lossy(&resume_out.stdout);

    assert!(
        resume_stdout.contains(r#""event":"stage_complete""#),
        "stage_complete not emitted: {}", resume_stdout
    );
    let complete: serde_json::Value = resume_stdout.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .find(|v: &serde_json::Value| v["event"] == "stage_complete")
        .expect("no stage_complete");
    assert_eq!(complete["stage"], "a");
    assert_eq!(complete["outputs"]["verdict"], "ok");
}

#[test]
fn test_stage_invoke_includes_declared_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    let invoke_line = stdout.lines()
        .find(|l| l.contains(r#""event":"stage_invoke""#))
        .expect("no stage_invoke");
    let event: serde_json::Value = serde_json::from_str(invoke_line).unwrap();

    // BASIC_TL stage a declares: out: verdict as value
    let outputs = event["outputs"].as_array()
        .expect("outputs field missing from stage_invoke");
    assert_eq!(outputs[0]["name"], "verdict");
    assert_eq!(outputs[0]["kind"], "value");
}

#[test]
fn test_api_driver_no_key_does_not_emit_pipeline_start() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "anthropic"])
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("THRULINE_DEFAULT_MODEL")
        .output().unwrap();

    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(r#""event":"pipeline_start""#),
        "pipeline_start emitted before key check: {}", stdout
    );
}

#[test]
fn test_resume_wrong_stage_emits_stage_error() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let run_out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let run_id = serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(&run_out.stdout).lines().next().unwrap()
    ).unwrap()["run_id"].as_str().unwrap().to_string();

    // Resume with wrong stage name
    let out = thruline()
        .args(["resume", &run_id, "--stage", "wrongstage"])
        .output().unwrap();
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(r#""event":"stage_error""#),
        "stage_error not emitted: {}", stdout
    );
}

#[test]
fn test_mock_driver_runs_full_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    // Write mock responses
    let mock = serde_json::json!({
        "a": { "verdict": "ok" },
        "b": {}
    });
    let mock_path = dir.path().join("mock.json");
    std::fs::write(&mock_path, mock.to_string()).unwrap();

    let out = thruline()
        .args([
            "run", tl.to_str().unwrap(),
            "--driver", "mock",
            "--mock-file", mock_path.to_str().unwrap(),
        ])
        .output().unwrap();

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(r#""event":"pipeline_done""#), "no pipeline_done: {}", stdout);
}

#[test]
fn test_graph_renders_flowchart() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "flow.line", BASIC_TL);

    let out = thruline().args(["graph", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // BASIC_TL has stages a and b
    assert!(stdout.contains("[a]"), "expected [a] in output: {}", stdout);
    assert!(stdout.contains("[b]"), "expected [b] in output: {}", stdout);
}

#[test]
fn test_graph_missing_file_errors() {
    let out = thruline().args(["graph", "/nonexistent/pipeline.line"]).output().unwrap();
    assert!(!out.status.success());
}
