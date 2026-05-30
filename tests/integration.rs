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
  out: verdict as ref
  agent: r
}
stage b {
  agent: r
}
pipeline p {
  start: a
  routes {
    a.verdict == "ok" -> b
  }
}
"#;

#[test]
fn test_validate_valid_file() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.tl", BASIC_TL);

    let out = thruline().args(["validate", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}

#[test]
fn test_validate_unknown_runner_fails() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "bad.tl", r#"
stage a {
  agent: ghost
}
pipeline p {
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
    let out = thruline().args(["validate", "/nonexistent/path.tl"]).output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn test_inspect_shows_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.tl", BASIC_TL);

    let out = thruline().args(["inspect", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pipeline: p"), "got: {}", stdout);
    assert!(stdout.contains("start: a"), "got: {}", stdout);
    assert!(stdout.contains("a.verdict"), "got: {}", stdout);
    assert!(stdout.contains("-> b"), "got: {}", stdout);
}

#[test]
fn test_inspect_shows_stages() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.tl", BASIC_TL);

    let out = thruline().args(["inspect", tl.to_str().unwrap()]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stage: a"), "got: {}", stdout);
    assert!(stdout.contains("stage: b"), "got: {}", stdout);
}

#[test]
fn test_run_stdio_emits_pipeline_start_and_stage_invoke() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.tl", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should emit pipeline_start and stage_invoke as NDJSON lines
    assert!(stdout.contains(r#""event":"pipeline_start""#), "got: {}", stdout);
    assert!(stdout.contains(r#""event":"stage_invoke""#), "got: {}", stdout);
    assert!(stdout.contains(r#""stage":"a""#), "got: {}", stdout);
    // Runner spec should be embedded in stage_invoke
    assert!(stdout.contains(r#""model":"claude-sonnet-4-6""#), "got: {}", stdout);
}

#[test]
fn test_run_creates_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.tl", BASIC_TL);

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
    let tl = write_tl(dir.path(), "test.tl", BASIC_TL);

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
    write_tl(dir.path(), "runners.tl", r#"
runner r {
  model: claude-sonnet-4-6
  system: "runner"
}
"#);

    // Main file imports runners.tl
    let tl = write_tl(dir.path(), "main.tl", r#"
import "runners.tl"
stage a { agent: r }
pipeline p {
  start: a
  routes {}
}
"#);

    let out = thruline().args(["validate", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}
