use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::runtime::artifact::ArtifactStore;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RunStatus {
    Running,
    AwaitingResume {
        stage: String,
        /// Present when the route that led here had a [*] or [*N] spec.
        /// None = no hint (single agent). Some(None) = [*]. Some(Some(N)) = [*N].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parallel: Option<Option<u32>>,
    },
    /// Waiting for parallel run blocks to complete. `pending_runs` shrinks as each run reports back.
    ParallelAwait { stage: String, pending_runs: Vec<String> },
    Done,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    pub run_id:    String,
    pub pipeline:  String,
    pub line_file:   PathBuf,
    pub status:    RunStatus,
    pub artifacts: ArtifactStore,
    pub history:   Vec<String>,
    #[serde(default)]
    pub visit_counts: HashMap<String, u32>,
    pub started:   DateTime<Utc>,
    pub updated:   DateTime<Utc>,
}

impl RunState {
    pub fn new(run_id: String, pipeline: String, line_file: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            run_id,
            pipeline,
            line_file,
            status: RunStatus::Running,
            artifacts: ArtifactStore::new(),
            history: Vec::new(),
            visit_counts: HashMap::new(),
            started: now,
            updated: now,
        }
    }

    pub fn run_dir(&self) -> PathBuf {
        runs_dir().join(&self.run_id)
    }

    pub fn state_path(&self) -> PathBuf {
        self.run_dir().join("state.json")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.run_dir().join("artifacts")
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        self.updated = Utc::now();
        let dir = self.run_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::create_dir_all(self.artifacts_dir())?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(self.state_path(), json)?;
        Ok(())
    }

    pub fn load(run_id: &str) -> anyhow::Result<Self> {
        let path = runs_dir().join(run_id).join("state.json");
        let json = std::fs::read_to_string(&path)
            .map_err(|_| anyhow::anyhow!("run '{}' not found", run_id))?;
        Ok(serde_json::from_str(&json)?)
    }
}

pub fn runs_dir() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".thruline")
        .join("runs")
}

pub fn list_runs() -> anyhow::Result<Vec<RunState>> {
    let dir = runs_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut runs = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let state_path = entry.path().join("state.json");
        if state_path.exists() {
            if let Ok(json) = std::fs::read_to_string(&state_path) {
                if let Ok(state) = serde_json::from_str::<RunState>(&json) {
                    runs.push(state);
                }
            }
        }
    }
    runs.sort_by(|a, b| b.started.cmp(&a.started));
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_new_has_running_status() {
        let state = RunState::new(
            "test-run-123".to_string(),
            "feature-dev".to_string(),
            "/tmp/test.line".into(),
        );
        assert_eq!(state.run_id, "test-run-123");
        assert_eq!(state.pipeline, "feature-dev");
        assert!(matches!(state.status, RunStatus::Running));
        assert!(state.history.is_empty());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let state = RunState::new(
            "r1".to_string(),
            "p1".to_string(),
            "/tmp/test.line".into(),
        );
        let json = serde_json::to_string(&state).unwrap();
        let back: RunState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.run_id, "r1");
        assert_eq!(back.pipeline, "p1");
        assert!(matches!(back.status, RunStatus::Running));
    }

    #[test]
    fn test_awaiting_resume_status_roundtrip() {
        let mut state = RunState::new("r".to_string(), "p".to_string(), "/t.line".into());
        state.status = RunStatus::AwaitingResume { stage: "interview".to_string(), parallel: None };
        let json = serde_json::to_string(&state).unwrap();
        let back: RunState = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.status, RunStatus::AwaitingResume { stage, .. } if stage == "interview"));
    }

    #[test]
    fn test_run_dir_path() {
        let state = RunState::new("abc123".to_string(), "p".to_string(), "/t.line".into());
        let dir = state.run_dir();
        assert!(dir.to_string_lossy().contains("abc123"));
        assert!(dir.to_string_lossy().contains(".thruline"));
    }

    #[test]
    fn test_list_runs_empty_when_no_dir() {
        // Just verify it doesn't panic when runs dir doesn't exist
        // (it may exist on developer machines — we just check it returns Ok)
        let result = list_runs();
        assert!(result.is_ok());
    }
}
