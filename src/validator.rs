// src/validator.rs
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use crate::ast::*;

#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("stage '{stage}' references unknown runner '{runner}'")]
    UnknownRunner { stage: String, runner: String },
    #[error("route references unknown stage '{0}'")]
    UnknownStage(String),
    #[error("route predicate references unknown artifact '{stage}.{artifact}'")]
    UnknownArtifact { stage: String, artifact: String },
    #[error("parallel fan-out for stage '{0}' has no matching fan-in route")]
    UnpairedFanOut(String),
    #[error("concurrency limit must be >= 1 in fan-out for stage '{0}'")]
    InvalidConcurrencyLimit(String),
    #[error("circular import: {0}")]
    CircularImport(String),
    #[error("duplicate name '{0}'")]
    DuplicateName(String),
    #[error("duplicate config block — only one config block allowed per file")]
    DuplicateConfig,
    #[error("stage '{stage}' run '{run}' references unknown runner '{runner}'")]
    UnknownRunnerInRun { stage: String, run: String, runner: String },
}

#[derive(Debug)]
pub struct Warning(pub String);

pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<Warning>,
}

pub fn validate(items: &[TlItem]) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Build index maps
    let mut runner_names: HashSet<String>  = HashSet::new();
    let mut stage_names:  HashSet<String>  = HashSet::new();
    let mut pipeline_names: HashSet<String> = HashSet::new();
    let mut config_seen = false;
    // stage name -> set of output artifact names
    let mut stage_outputs: HashMap<String, HashSet<String>> = HashMap::new();

    // First pass: build indexes, check duplicates
    for item in items {
        match item {
            TlItem::Config(_) => {
                if config_seen {
                    errors.push(ValidationError::DuplicateConfig);
                }
                config_seen = true;
            }
            TlItem::Runner(r) => {
                if !runner_names.insert(r.name.clone()) {
                    errors.push(ValidationError::DuplicateName(r.name.clone()));
                }
            }
            TlItem::Stage(s) => {
                if !stage_names.insert(s.name.clone()) {
                    errors.push(ValidationError::DuplicateName(s.name.clone()));
                }
                let outs: HashSet<String> = s.outputs.iter().map(|a| a.name.clone()).collect();
                stage_outputs.insert(s.name.clone(), outs);
            }
            TlItem::Pipeline(p) => {
                if !pipeline_names.insert(p.name.clone()) {
                    errors.push(ValidationError::DuplicateName(p.name.clone()));
                }
            }
            TlItem::Import(_) => {}
        }
    }

    // Second pass: validate stage runner refs and run block runner refs
    for item in items {
        if let TlItem::Stage(s) = item {
            if let Some(r) = &s.runner {
                if !runner_names.contains(r) {
                    errors.push(ValidationError::UnknownRunner {
                        stage: s.name.clone(),
                        runner: r.clone(),
                    });
                }
            }
            for run in &s.runs {
                if let Some(r) = &run.runner {
                    if !runner_names.contains(r) {
                        errors.push(ValidationError::UnknownRunnerInRun {
                            stage: s.name.clone(),
                            run: run.name.clone(),
                            runner: r.clone(),
                        });
                    }
                }
            }
        }
    }

    // Third pass: validate pipeline routes
    for item in items {
        let TlItem::Pipeline(p) = item else { continue };

        let mut fan_out_stages: HashSet<String> = HashSet::new();
        let mut fan_in_stages:  HashSet<String> = HashSet::new();
        let mut referenced_stages: HashSet<String> = HashSet::new();

        // Check start stage
        if !stage_names.contains(&p.start) {
            errors.push(ValidationError::UnknownStage(p.start.clone()));
        }
        referenced_stages.insert(p.start.clone());

        for route in &p.routes {
            match &route.source {
                RouteSource::Stage(s) => {
                    if !stage_names.contains(s) {
                        errors.push(ValidationError::UnknownStage(s.clone()));
                    }
                    referenced_stages.insert(s.clone());
                }
                RouteSource::FanIn(s) => {
                    if !stage_names.contains(s) {
                        errors.push(ValidationError::UnknownStage(s.clone()));
                    }
                    referenced_stages.insert(s.clone());
                    fan_in_stages.insert(s.clone());
                }
                RouteSource::Predicate { stage, artifact, .. } => {
                    if !stage_names.contains(stage) {
                        errors.push(ValidationError::UnknownStage(stage.clone()));
                    } else if let Some(outs) = stage_outputs.get(stage) {
                        if !outs.contains(artifact) {
                            errors.push(ValidationError::UnknownArtifact {
                                stage: stage.clone(),
                                artifact: artifact.clone(),
                            });
                        }
                    }
                    referenced_stages.insert(stage.clone());
                }
            }

            // Check target
            let target = &route.target.stage;
            if !stage_names.contains(target) {
                errors.push(ValidationError::UnknownStage(target.clone()));
            }
            referenced_stages.insert(target.clone());

            // Check fan-out
            if let Some(spec) = &route.target.parallel_spec {
                if spec.limit == Some(0) {
                    errors.push(ValidationError::InvalidConcurrencyLimit(target.clone()));
                }
                fan_out_stages.insert(target.clone());
            }
        }

        // Every fan-out must have a fan-in
        for fo_stage in &fan_out_stages {
            if !fan_in_stages.contains(fo_stage) {
                errors.push(ValidationError::UnpairedFanOut(fo_stage.clone()));
            }
        }

        // Warn on unreachable stages
        for name in &stage_names {
            if !referenced_stages.contains(name) {
                warnings.push(Warning(format!(
                    "stage '{}' is unreachable in pipeline '{}'",
                    name, p.name
                )));
            }
        }
    }

    ValidationResult { errors, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(name: &str) -> TlItem {
        TlItem::Runner(RunnerDecl {
            name: name.to_string(),
            model: "claude-sonnet-4-6".to_string(),
            system: Some(PromptSource::Inline("system".to_string())),
            tools: vec![],
            temperature: None,
            max_tokens: None,
        })
    }

    fn stage(name: &str, runner_name: &str, outputs: &[&str]) -> TlItem {
        TlItem::Stage(StageDecl {
            name: name.to_string(),
            inputs: vec![],
            outputs: outputs.iter().map(|o| ArtifactDecl {
                name: o.to_string(),
                optional: false,
                kind: ArtifactKind::Ref,
                seed_path: None,
            }).collect(),
            runner: Some(runner_name.to_string()),
            prompt: None,
            runs: vec![],
        })
    }

    fn unconditional(from: &str, to: &str) -> Route {
        Route {
            source: RouteSource::Stage(from.to_string()),
            target: RouteTarget { stage: to.to_string(), parallel_spec: None },
            parallel: false,
        }
    }

    fn pipeline(name: &str, start: &str, routes: Vec<Route>) -> TlItem {
        TlItem::Pipeline(PipelineDecl {
            name: name.to_string(),
            start: start.to_string(),
            routes,
        })
    }

    #[test]
    fn test_valid_simple_pipeline() {
        let items = vec![
            runner("r"),
            stage("a", "r", &["out"]),
            stage("b", "r", &[]),
            pipeline("p", "a", vec![unconditional("a", "b")]),
        ];
        let result = validate(&items);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn test_unknown_runner() {
        let items = vec![
            stage("a", "ghost", &[]),  // references runner "ghost" which doesn't exist
            pipeline("p", "a", vec![]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e,
            ValidationError::UnknownRunner { runner, .. } if runner == "ghost")));
    }

    #[test]
    fn test_stage_without_runner_is_valid() {
        let items = vec![
            TlItem::Stage(StageDecl {
                name: "a".to_string(),
                inputs: vec![],
                outputs: vec![],
                runner: None,
                prompt: None,
                runs: vec![],
            }),
            pipeline("p", "a", vec![]),
        ];
        let result = validate(&items);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn test_unknown_stage_in_route() {
        let items = vec![
            runner("r"),
            stage("a", "r", &[]),
            pipeline("p", "a", vec![unconditional("a", "nonexistent")]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e,
            ValidationError::UnknownStage(s) if s == "nonexistent")));
    }

    #[test]
    fn test_unknown_artifact_in_predicate() {
        let items = vec![
            runner("r"),
            stage("a", "r", &["verdict"]),
            stage("b", "r", &[]),
            pipeline("p", "a", vec![Route {
                source: RouteSource::Predicate {
                    stage: "a".to_string(),
                    artifact: "ghost_artifact".to_string(),
                    op: CompareOp::Eq,
                    value: "x".to_string(),
                },
                target: RouteTarget { stage: "b".to_string(), parallel_spec: None },
                parallel: false,
            }]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e,
            ValidationError::UnknownArtifact { artifact, .. } if artifact == "ghost_artifact")));
    }

    #[test]
    fn test_unpaired_fan_out() {
        let items = vec![
            runner("r"),
            stage("a", "r", &[]),
            stage("b", "r", &[]),
            pipeline("p", "a", vec![Route {
                source: RouteSource::Stage("a".to_string()),
                target: RouteTarget {
                    stage: "b".to_string(),
                    parallel_spec: Some(ParallelSpec { limit: Some(2) }),
                },
                parallel: true,
            }]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e, ValidationError::UnpairedFanOut(s) if s == "b")));
    }

    #[test]
    fn test_invalid_concurrency_limit() {
        let items = vec![
            runner("r"),
            stage("a", "r", &[]),
            stage("b", "r", &[]),
            stage("c", "r", &[]),
            pipeline("p", "a", vec![
                Route {
                    source: RouteSource::Stage("a".to_string()),
                    target: RouteTarget {
                        stage: "b".to_string(),
                        parallel_spec: Some(ParallelSpec { limit: Some(0) }),
                    },
                    parallel: true,
                },
                Route {
                    source: RouteSource::FanIn("b".to_string()),
                    target: RouteTarget { stage: "c".to_string(), parallel_spec: None },
                    parallel: false,
                },
            ]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e, ValidationError::InvalidConcurrencyLimit(s) if s == "b")));
    }

    #[test]
    fn test_duplicate_runner_name() {
        let items = vec![runner("r"), runner("r")];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e, ValidationError::DuplicateName(n) if n == "r")));
    }

    #[test]
    fn test_missing_model_is_valid() {
        // model is optional — runners without a model declaration are valid
        let items = vec![TlItem::Runner(RunnerDecl {
            name: "r".to_string(),
            model: "".to_string(),
            system: None,
            tools: vec![],
            temperature: None,
            max_tokens: None,
        })];
        let result = validate(&items);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn test_valid_fan_out_fan_in_pair() {
        let items = vec![
            runner("r"),
            stage("a", "r", &[]),
            stage("b", "r", &[]),
            stage("c", "r", &[]),
            pipeline("p", "a", vec![
                Route {
                    source: RouteSource::Stage("a".to_string()),
                    target: RouteTarget {
                        stage: "b".to_string(),
                        parallel_spec: Some(ParallelSpec { limit: Some(3) }),
                    },
                    parallel: true,
                },
                Route {
                    source: RouteSource::FanIn("b".to_string()),
                    target: RouteTarget { stage: "c".to_string(), parallel_spec: None },
                    parallel: false,
                },
            ]),
        ];
        let result = validate(&items);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn test_retry_cycle_is_not_an_error() {
        // A->A retry loop should be valid (no error)
        let items = vec![
            runner("r"),
            stage("a", "r", &["verdict"]),
            pipeline("p", "a", vec![
                Route {
                    source: RouteSource::Predicate {
                        stage: "a".to_string(),
                        artifact: "verdict".to_string(),
                        op: CompareOp::Eq,
                        value: "retry".to_string(),
                    },
                    target: RouteTarget { stage: "a".to_string(), parallel_spec: None },
                    parallel: false,
                },
            ]),
        ];
        let result = validate(&items);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn test_duplicate_config_error() {
        let items = vec![
            TlItem::Config(ConfigDecl { model: Some("claude-sonnet-4-6".to_string()) }),
            TlItem::Config(ConfigDecl { model: None }),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e, ValidationError::DuplicateConfig)));
    }

    #[test]
    fn test_run_block_unknown_runner() {
        let items = vec![
            runner("r"),
            TlItem::Stage(StageDecl {
                name: "a".to_string(),
                inputs: vec![],
                outputs: vec![],
                runner: Some("r".to_string()),
                prompt: None,
                runs: vec![
                    RunDecl {
                        name: "fast".to_string(),
                        runner: Some("ghost".to_string()),
                        prompt: None,
                        outputs: vec![],
                    },
                ],
            }),
            pipeline("p", "a", vec![]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e,
            ValidationError::UnknownRunnerInRun { run, runner, .. }
            if run == "fast" && runner == "ghost")));
    }

    #[test]
    fn test_run_block_valid_runner_ref() {
        let items = vec![
            runner("analyst"),
            runner("critic"),
            TlItem::Stage(StageDecl {
                name: "review".to_string(),
                inputs: vec![],
                outputs: vec![],
                runner: Some("analyst".to_string()),
                prompt: None,
                runs: vec![
                    RunDecl { name: "fast".to_string(), runner: None, prompt: None, outputs: vec![] },
                    RunDecl { name: "deep".to_string(), runner: Some("critic".to_string()), prompt: None, outputs: vec![] },
                ],
            }),
            pipeline("p", "review", vec![]),
        ];
        let result = validate(&items);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }
}
