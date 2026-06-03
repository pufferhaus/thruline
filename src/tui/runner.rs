use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::Sender;

/// Spawn a `thruline run` child process and stream each NDJSON line to `tx` as `(run_id, line)`.
/// The run_id is extracted from the first `pipeline_start` event.
pub async fn spawn_run(
    file: &Path,
    driver: &str,
    inputs: &[String],
    pipeline: Option<&str>,
    tx: Sender<(String, String)>,
) -> anyhow::Result<()> {
    let bin = std::env::current_exe()?;
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("run").arg(file).arg("--driver").arg(driver);
    for input in inputs {
        cmd.arg("--input").arg(input);
    }
    if let Some(name) = pipeline {
        cmd.arg("--pipeline").arg(name);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut run_id = String::from("unknown");

    while let Ok(Some(line)) = lines.next_line().await {
        // Try to extract run_id from pipeline_start
        if run_id == "unknown" {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v["event"] == "pipeline_start" {
                    if let Some(id) = v["run_id"].as_str() {
                        run_id = id.to_string();
                    }
                }
            }
        }
        let _ = tx.send((run_id.clone(), line)).await;
    }

    child.wait().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_spawn_run_streams_pipeline_start() {
        // Find the thruline binary adjacent to test binary
        let bin = std::env::current_exe().unwrap();
        let bin_dir = bin.parent().unwrap();
        let thruline_bin = bin_dir.join("thruline");
        if !thruline_bin.exists() {
            // Skip if not built
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let content = r#"
runner r { model: claude-sonnet-4-6  system: "test" }
stage a { out: v as value  runner: r }
thruline p { start: a  routes {} }
"#;
        let tl_path = dir.path().join("test.line");
        std::fs::write(&tl_path, content).unwrap();

        let (tx, mut rx) = mpsc::channel(64);
        let _ = spawn_run(&tl_path, "stdio", &[], None, tx).await;

        let mut events = vec![];
        while let Ok((_id, line)) = rx.try_recv() {
            events.push(line);
        }
        assert!(!events.is_empty(), "no events received");
        assert!(events[0].contains("pipeline_start"), "first event: {}", events[0]);
    }
}
