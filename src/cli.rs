// src/cli.rs
use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use crate::ast::{CompareOp, RouteSource, TlItem};
use crate::runtime::state::{list_runs, RunState, RunStatus};
use crate::validator;

#[derive(Parser)]
#[command(name = "thruline", about = "Deterministic multiagent pipeline runner")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Parse and statically validate a .line file
    Validate { file: PathBuf },
    /// Print the pipeline graph as an ASCII representation
    Inspect { file: PathBuf },
    /// List all runs
    Runs,
    /// Show current status of a run
    Status { run_id: String },
    /// Execute a pipeline
    Run {
        file: PathBuf,
        #[arg(long)]
        pipeline: Option<String>,
        #[arg(long, default_value = "stdio")]
        driver: String,
        /// Pipeline inputs: key=value (prefix file:// for file artifacts)
        #[arg(long = "input")]
        inputs: Vec<String>,
    },
    /// Feed agent output back to a suspended run
    Resume {
        run_id: String,
        #[arg(long)]
        stage: String,
        /// Artifact values: key=value (prefix file:// for file artifacts)
        #[arg(long = "artifact")]
        artifacts: Vec<String>,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Validate { file }  => cmd_validate(&file),
        Commands::Inspect  { file }  => cmd_inspect(&file),
        Commands::Runs               => cmd_runs(),
        Commands::Status { run_id }  => cmd_status(&run_id),
        Commands::Run { file, pipeline, driver, inputs } => cmd_run(&file, pipeline.as_deref(), &driver, &inputs).await,
        Commands::Resume { run_id, stage, artifacts } => cmd_resume(&run_id, &stage, &artifacts).await,
    }
}

// ── Import resolution ─────────────────────────────────────────────────────

/// Load a .line file and all its imports recursively, deduplicating by canonical path.
/// Import items are consumed (not kept in output); all other items are returned flat.
pub fn load_items(entry: &Path) -> anyhow::Result<Vec<TlItem>> {
    let mut items = Vec::new();
    let mut seen  = HashSet::new();
    load_recursive(entry, &mut items, &mut seen)?;
    Ok(items)
}

fn load_recursive(
    path: &Path,
    items: &mut Vec<TlItem>,
    seen: &mut HashSet<PathBuf>,
) -> anyhow::Result<()> {
    let abs = path.canonicalize()
        .map_err(|_| anyhow::anyhow!("file not found: {}", path.display()))?;
    if !seen.insert(abs.clone()) {
        return Ok(());
    }
    let parsed = crate::parser::parse_file(path)?;
    let base   = abs.parent().unwrap_or(Path::new("."));
    for item in parsed {
        if let TlItem::Import(rel) = &item {
            let imported = base.join(rel);
            load_recursive(&imported, items, seen)?;
        } else {
            items.push(item);
        }
    }
    Ok(())
}

// ── Commands ──────────────────────────────────────────────────────────────

fn cmd_validate(file: &Path) -> anyhow::Result<()> {
    let items  = load_items(file)?;
    let mut result = validator::validate(&items);

    // Check system: file(...) paths exist relative to the .line file
    let base = file.parent().unwrap_or(Path::new("."));
    for item in &items {
        if let TlItem::Runner(r) = item {
            if let Some(crate::ast::PromptSource::File(rel)) = &r.system {
                if !base.join(rel).exists() {
                    result.errors.push(validator::ValidationError::SystemFileNotFound {
                        name: r.name.clone(),
                        path: rel.clone(),
                    });
                }
            }
        }
    }

    for w in &result.warnings {
        eprintln!("warning: {}", w.0);
    }
    if result.errors.is_empty() {
        println!("ok");
        Ok(())
    } else {
        for e in &result.errors {
            eprintln!("error: {}", e);
        }
        anyhow::bail!("{} validation error(s)", result.errors.len())
    }
}

fn cmd_inspect(file: &Path) -> anyhow::Result<()> {
    let items = load_items(file)?;
    for item in &items {
        if let TlItem::Pipeline(p) = item {
            println!("pipeline: {}", p.name);
            if !p.inputs.is_empty() {
                println!("  inputs:");
                for input in &p.inputs {
                    let opt = if input.optional { "?" } else { "" };
                    let kind = match input.kind {
                        crate::ast::ArtifactKind::Path  => "path",
                        crate::ast::ArtifactKind::Value => "value",
                    };
                    println!("    {}{} as {}", input.name, opt, kind);
                }
            }
            println!("  start: {}", p.start);
            println!("  routes:");
            for route in &p.routes {
                let src = match &route.source {
                    RouteSource::Stage(s) => s.clone(),
                    RouteSource::FanIn(s) => format!("{}[*]", s),
                    RouteSource::Predicate { stage, artifact, op, value } => {
                        let op_str = match op {
                            CompareOp::Eq => "==",
                            CompareOp::Ne => "!=",
                        };
                        format!("{}.{} {} \"{}\"", stage, artifact, op_str, value)
                    }
                };
                let tgt = if let Some(spec) = &route.target.parallel_spec {
                    let n = spec.limit.map(|l| l.to_string()).unwrap_or_default();
                    format!("{}[*{}]", route.target.stage, n)
                } else {
                    route.target.stage.clone()
                };
                println!("    {} -> {}", src, tgt);
            }
            println!();
        }
    }
    for item in &items {
        if let TlItem::Stage(s) = item {
            if s.runs.is_empty() {
                let runner_label = s.runner.as_deref().unwrap_or("default");
                println!("stage: {}  (runner: {})", s.name, runner_label);
            } else {
                let default_label = s.runner.as_deref().map(|r| format!(", default runner: {}", r)).unwrap_or_default();
                println!("stage: {}  (parallel runs{})", s.name, default_label);
                for run in &s.runs {
                    let run_runner = run.runner.as_deref()
                        .or(s.runner.as_deref())
                        .unwrap_or("default");
                    println!("  run: {}  (runner: {})", run.name, run_runner);
                    for a in &run.outputs {
                        let kind = match a.kind {
                            crate::ast::ArtifactKind::Path  => "path",
                            crate::ast::ArtifactKind::Value => "value",
                        };
                        println!("    out: {} as {}", a.name, kind);
                    }
                }
            }
            for a in &s.outputs {
                let kind = match a.kind {
                    crate::ast::ArtifactKind::Path  => "path",
                    crate::ast::ArtifactKind::Value => "value",
                };
                println!("  out: {} as {}", a.name, kind);
            }
        }
    }
    Ok(())
}

fn cmd_runs() -> anyhow::Result<()> {
    let runs = list_runs()?;
    if runs.is_empty() {
        println!("no runs found");
        return Ok(());
    }
    println!("{:<20} {:<20} {:<20} {}", "RUN ID", "PIPELINE", "STATUS", "STARTED");
    for run in runs {
        let status = match &run.status {
            RunStatus::Running => "running".to_string(),
            RunStatus::AwaitingResume { stage } => format!("awaiting:{}", stage),
            RunStatus::ParallelAwait { stage, remaining } => {
                format!("parallel:{}:{}", stage, remaining)
            }
            RunStatus::Done        => "done".to_string(),
            RunStatus::Failed(e)   => format!("failed:{}", e),
        };
        println!(
            "{:<20} {:<20} {:<20} {}",
            run.run_id,
            run.pipeline,
            status,
            run.started.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

fn cmd_status(run_id: &str) -> anyhow::Result<()> {
    let state = RunState::load(run_id)?;
    println!("run:      {}", state.run_id);
    println!("pipeline: {}", state.pipeline);
    println!("file:     {}", state.line_file.display());
    println!("status:   {:?}", state.status);
    println!("history:  {}", state.history.join(" \u{2192} "));
    println!("updated:  {}", state.updated);
    println!("\nartifacts:");
    let mut keys: Vec<_> = state.artifacts.file_keys().collect();
    keys.sort();
    for key in keys {
        println!("  {} (file) = {}", key, state.artifacts.get_file(key).unwrap().display());
    }
    let mut rkeys: Vec<_> = state.artifacts.ref_keys().collect();
    rkeys.sort();
    for key in rkeys {
        println!("  {} (ref)  = {}", key, state.artifacts.get_ref(key).unwrap_or(""));
    }
    Ok(())
}

pub async fn cmd_run(
    file: &Path,
    pipeline_name: Option<&str>,
    driver_name: &str,
    input_args: &[String],
) -> anyhow::Result<()> {
    use crate::runtime::Runtime;
    use uuid::Uuid;

    let items  = load_items(file)?;
    let result = validator::validate(&items);
    if !result.errors.is_empty() {
        for e in &result.errors { eprintln!("error: {}", e); }
        anyhow::bail!("validation failed");
    }

    let pipeline = match pipeline_name {
        Some(n) => n.to_string(),
        None => {
            let pipelines: Vec<_> = items.iter().filter_map(|i| {
                if let TlItem::Pipeline(p) = i { Some(p.name.clone()) } else { None }
            }).collect();
            match pipelines.len() {
                0 => anyhow::bail!("no pipeline defined in {}", file.display()),
                1 => pipelines[0].clone(),
                _ => anyhow::bail!("multiple pipelines — use --pipeline <name>"),
            }
        }
    };

    let run_id = Uuid::new_v4().to_string()[..8].to_string();
    let mut state = RunState::new(run_id.clone(), pipeline.clone(), file.canonicalize()?);

    // Pre-seed pipeline inputs into the artifact store under the input.* namespace
    for arg in input_args {
        let (k, v) = arg
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--input must be key=value, got '{}'", arg))?;
        let store_key = format!("input.{}", k);
        if v.starts_with("file://") {
            state.artifacts.set_file(&store_key, &v[7..]);
        } else {
            state.artifacts.set_ref(&store_key, v);
        }
    }

    let config_model = items.iter().find_map(|i| {
        if let TlItem::Config(c) = i { c.model.clone() } else { None }
    });

    let mut runtime = Runtime::new(state, items);

    // Validate required pipeline inputs are present before first advance
    runtime.check_pipeline_inputs()?;

    match driver_name {
        "stdio" => {
            crate::events::ThrulineEvent::PipelineStart {
                run_id: run_id.clone(),
                ts: chrono::Utc::now(),
                pipeline: pipeline.clone(),
                inputs: serde_json::Value::Null,
            }.emit();
            let driver = crate::driver::stdio::StdioDriver;
            if let Err(e) = runtime.advance(&driver).await {
                crate::events::ThrulineEvent::PipelineError {
                    run_id: run_id.clone(),
                    ts: chrono::Utc::now(),
                    stage: runtime.state.history.last().cloned().unwrap_or_else(|| "unknown".to_string()),
                    error: e.to_string(),
                }.emit();
                return Err(e);
            }
        }
        "api" => {
            let driver = crate::driver::api::ApiDriver::from_env(config_model)?;
            crate::events::ThrulineEvent::PipelineStart {
                run_id: run_id.clone(),
                ts: chrono::Utc::now(),
                pipeline: pipeline.clone(),
                inputs: serde_json::Value::Null,
            }.emit();
            if let Err(e) = runtime.advance(&driver).await {
                crate::events::ThrulineEvent::PipelineError {
                    run_id: run_id.clone(),
                    ts: chrono::Utc::now(),
                    stage: runtime.state.history.last().cloned().unwrap_or_else(|| "unknown".to_string()),
                    error: e.to_string(),
                }.emit();
                return Err(e);
            }
        }
        other => anyhow::bail!("unknown driver '{}' \u{2014} use stdio or api", other),
    }

    Ok(())
}

pub async fn cmd_resume(
    run_id: &str,
    stage_name: &str,
    artifact_args: &[String],
) -> anyhow::Result<()> {
    use crate::runtime::{state::RunState, Runtime};

    let state = RunState::load(run_id)?;
    let file  = state.line_file.clone();
    let items = load_items(&file)?;
    let mut runtime = Runtime::new(state, items);

    let outputs: Vec<(String, String, bool)> = artifact_args
        .iter()
        .map(|arg| -> anyhow::Result<(String, String, bool)> {
            let (k, v) = arg
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("artifact arg must be key=value, got '{}'", arg))?;
            let is_file = v.starts_with("file://");
            let val = if is_file { v[7..].to_string() } else { v.to_string() };
            Ok((k.to_string(), val, is_file))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    if let Err(e) = runtime.resume_stage(stage_name, outputs) {
        crate::events::ThrulineEvent::StageError {
            run_id: run_id.to_string(),
            ts: chrono::Utc::now(),
            stage: stage_name.to_string(),
            error: e.to_string(),
        }
        .emit();
        return Err(e);
    }

    if matches!(runtime.state.status, RunStatus::AwaitingResume { .. }) {
        let driver = crate::driver::stdio::StdioDriver;
        if let Err(e) = runtime.advance(&driver).await {
            crate::events::ThrulineEvent::PipelineError {
                run_id: run_id.to_string(),
                ts: chrono::Utc::now(),
                stage: runtime.state.history.last()
                    .cloned().unwrap_or_else(|| "unknown".to_string()),
                error: e.to_string(),
            }
            .emit();
            return Err(e);
        }
    }

    Ok(())
}
