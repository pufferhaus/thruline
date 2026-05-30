use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RunnerSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub system: String,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ThrulineEvent {
    PipelineStart {
        run_id: String,
        ts: DateTime<Utc>,
        pipeline: String,
        inputs: serde_json::Value,
    },
    StageInvoke {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        runner: RunnerSpec,
        artifacts: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
    },
    StageComplete {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        outputs: serde_json::Value,
    },
    StageError {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        error: String,
    },
    RouteTaken {
        run_id: String,
        ts: DateTime<Utc>,
        from: String,
        to: String,
        predicate: String,
    },
    ParallelStart {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        count: usize,
        concurrency_limit: Option<u32>,
    },
    ParallelSlotOpen {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        slot: usize,
        total: usize,
    },
    ParallelDone {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        results: Vec<serde_json::Value>,
    },
    PipelineDone {
        run_id: String,
        ts: DateTime<Utc>,
        outputs: Vec<String>,
    },
    PipelineError {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        error: String,
    },
}

impl ThrulineEvent {
    pub fn emit(&self) {
        println!("{}", serde_json::to_string(self).expect("event serialization failed"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_start_serializes() {
        let ev = ThrulineEvent::PipelineStart {
            run_id: "abc".to_string(),
            ts: Utc::now(),
            pipeline: "feature-dev".to_string(),
            inputs: serde_json::Value::Null,
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains(r#""event":"pipeline_start""#));
        assert!(s.contains(r#""pipeline":"feature-dev""#));
    }

    #[test]
    fn test_stage_invoke_roundtrip() {
        let ev = ThrulineEvent::StageInvoke {
            run_id: "abc".to_string(),
            ts: Utc::now(),
            stage: "interview".to_string(),
            runner: RunnerSpec {
                name: "interviewer".to_string(),
                model: Some("claude-opus-4-8".to_string()),
                system: "You are an interviewer.".to_string(),
                tools: vec!["read_file".to_string()],
                temperature: Some(0.7),
                max_tokens: None,
            },
            artifacts: serde_json::json!({}),
            prompt: None,
        };
        let s = serde_json::to_string(&ev).unwrap();
        let back: ThrulineEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ThrulineEvent::StageInvoke { stage, .. } if stage == "interview"));
    }

    #[test]
    fn test_optional_fields_skipped_when_none() {
        let runner = RunnerSpec {
            name: "r".to_string(),
            model: None,
            system: "s".to_string(),
            tools: vec![],
            temperature: None,
            max_tokens: None,
        };
        let s = serde_json::to_string(&runner).unwrap();
        assert!(!s.contains("temperature"));
        assert!(!s.contains("max_tokens"));
        assert!(!s.contains("model"));
    }

    #[test]
    fn test_all_event_variants_serialize() {
        let run_id = "r1".to_string();
        let ts = Utc::now();
        let events: Vec<ThrulineEvent> = vec![
            ThrulineEvent::StageComplete {
                run_id: run_id.clone(), ts,
                stage: "a".to_string(),
                outputs: serde_json::json!({}),
            },
            ThrulineEvent::RouteTaken {
                run_id: run_id.clone(), ts,
                from: "a".to_string(), to: "b".to_string(),
                predicate: "always".to_string(),
            },
            ThrulineEvent::ParallelStart {
                run_id: run_id.clone(), ts,
                stage: "b".to_string(), count: 3,
                concurrency_limit: Some(2),
            },
            ThrulineEvent::PipelineDone {
                run_id: run_id.clone(), ts,
                outputs: vec!["notes.md".to_string()],
            },
        ];
        for ev in &events {
            let s = serde_json::to_string(ev).unwrap();
            assert!(s.contains(r#""event":"#), "missing event tag in: {}", s);
        }
    }
}
