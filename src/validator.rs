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
    #[error("runner '{0}' is missing required field 'system'")]
    MissingSystem(String),
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
    // stage name -> set of output artifact names
    let mut stage_outputs: HashMap<String, HashSet<String>> = HashMap::new();

    // First pass: build indexes, check duplicates, check required runner fields
    for item in items {
        match item {
            TlItem::Runner(r) => {
                if !runner_names.insert(r.name.clone()) {
                    errors.push(ValidationError::DuplicateName(r.name.clone()));
                }
                if matches!(&r.system, PromptSource::Inline(s) if s.is_empty()) {
                    errors.push(ValidationError::MissingSystem(r.name.clone()));
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

    // Second pass: validate stage agent refs
    for item in items {
        if let TlItem::Stage(s) = item {
            if !runner_names.contains(&s.agent) {
                errors.push(ValidationError::UnknownRunner {
                    stage: s.name.clone(),
                    runner: s.agent.clone(),
                });
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
            system: PromptSource::Inline("system".to_string()),
            tools: vec![],
            temperature: None,
            max_tokens: None,
        })
    }

    fn stage(name: &str, agent: &str, outputs: &[&str]) -> TlItem {
        TlItem::Stage(StageDecl {
            name: name.to_string(),
            inputs: vec![],
            outputs: outputs.iter().map(|o| ArtifactDecl {
                name: o.to_string(),
                optional: false,
                kind: ArtifactKind::Ref,
                seed_path: None,
            }).collect(),
            agent: agent.to_string(),
            prompt: None,
            format: None,
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
            stage("a", "ghost", &[]),
            pipeline("p", "a", vec![]),
        ];
        let result = validate(&items);
        assert!(result.errors.iter().any(|e| matches!(e,
            ValidationError::UnknownRunner { runner, .. } if runner == "ghost")));
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
            system: PromptSource::Inline("sys".to_string()),
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
}
