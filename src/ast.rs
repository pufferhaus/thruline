// src/ast.rs

#[derive(Debug, Clone)]
pub enum TlItem {
    Import(String),
    Config(ConfigDecl),
    Runner(RunnerDecl),
    Stage(StageDecl),
    Pipeline(PipelineDecl),
}

#[derive(Debug, Clone)]
pub struct ConfigDecl {
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunnerDecl {
    pub name: String,
    pub model: String,
    pub system: Option<PromptSource>,
    pub tools: Vec<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum PromptSource {
    File(String),
    Inline(String),
}

#[derive(Debug, Clone)]
pub struct RunDecl {
    pub name: String,
    pub runner: Option<String>,
    pub prompt: Option<PromptSource>,
    pub outputs: Vec<ArtifactDecl>,
}

#[derive(Debug, Clone)]
pub struct StageDecl {
    pub name: String,
    pub inputs: Vec<ArtifactDecl>,
    pub outputs: Vec<ArtifactDecl>,
    pub runner: Option<String>,
    pub prompt: Option<PromptSource>,
    pub format: Option<String>,
    pub runs: Vec<RunDecl>,
}

#[derive(Debug, Clone)]
pub struct ArtifactDecl {
    pub name: String,
    pub optional: bool,
    pub kind: ArtifactKind,
    pub seed_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactKind {
    File,
    Ref,
}

#[derive(Debug, Clone)]
pub struct PipelineDecl {
    pub name: String,
    pub start: String,
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub source: RouteSource,
    pub target: RouteTarget,
    pub parallel: bool,
}

#[derive(Debug, Clone)]
pub enum RouteSource {
    Stage(String),
    FanIn(String),
    Predicate {
        stage: String,
        artifact: String,
        op: CompareOp,
        value: String,
    },
}

#[derive(Debug, Clone)]
pub struct RouteTarget {
    pub stage: String,
    pub parallel_spec: Option<ParallelSpec>,
}

#[derive(Debug, Clone)]
pub struct ParallelSpec {
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompareOp {
    Eq,
    Ne,
}
