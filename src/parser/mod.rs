// src/parser/mod.rs
use pest::iterators::Pair;
use pest::Parser;
use pest_derive::Parser;
use std::path::Path;
use crate::ast::*;

#[derive(Parser)]
#[grammar = "parser/grammar.pest"]
pub struct TlParser;

pub fn parse_file(path: &Path) -> anyhow::Result<Vec<TlItem>> {
    let src = std::fs::read_to_string(path)?;
    parse_str(&src)
}

pub fn parse_str(src: &str) -> anyhow::Result<Vec<TlItem>> {
    let file = TlParser::parse(Rule::line_file, src)
        .map_err(|e| anyhow::anyhow!("parse error: {}", e))?
        .next()
        .unwrap();

    let mut items = Vec::new();
    for pair in file.into_inner() {
        match pair.as_rule() {
            Rule::import_decl   => items.push(parse_import(pair)),
            Rule::config_decl   => items.push(TlItem::Config(parse_config(pair))),
            Rule::runner_decl   => items.push(TlItem::Runner(parse_runner(pair))),
            Rule::stage_decl    => items.push(TlItem::Stage(parse_stage(pair))),
            Rule::pipeline_decl => items.push(TlItem::Pipeline(parse_pipeline(pair))),
            Rule::EOI           => {}
            _ => {}
        }
    }
    Ok(items)
}

fn parse_import(pair: Pair<Rule>) -> TlItem {
    let path = unquote(pair.into_inner().next().unwrap().as_str());
    TlItem::Import(path)
}

fn parse_config(pair: Pair<Rule>) -> ConfigDecl {
    let mut model = None;
    for field in pair.into_inner() {
        let mut fi = field.into_inner();
        let sub = fi.next().unwrap();
        if sub.as_rule() == Rule::config_model {
            let sv = sub.into_inner().next().unwrap();
            model = Some(parse_string_val(sv));
        }
    }
    ConfigDecl { model }
}

fn parse_runner(pair: Pair<Rule>) -> RunnerDecl {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut model = String::new();
    let mut system: Option<PromptSource> = None;
    let mut tools = Vec::new();
    let mut temperature = None;
    let mut max_tokens = None;

    for field in inner {
        // Each field pair is a runner_field, whose inner contains the specific sub-rule
        let mut fi = field.into_inner();
        let sub = fi.next().unwrap();
        match sub.as_rule() {
            Rule::runner_model => {
                let sv = sub.into_inner().next().unwrap();
                model = parse_string_val(sv);
            }
            Rule::runner_sys => {
                let pv = sub.into_inner().next().unwrap();
                system = Some(parse_prompt_val(pv));
            }
            Rule::runner_tools => {
                let tl = sub.into_inner().next().unwrap(); // tools_list
                tools = tl.into_inner().map(|t| t.as_str().to_string()).collect();
            }
            Rule::runner_temp => {
                temperature = Some(sub.into_inner().next().unwrap().as_str().parse().unwrap());
            }
            Rule::runner_maxt => {
                max_tokens = Some(sub.into_inner().next().unwrap().as_str().parse().unwrap());
            }
            _ => {}
        }
    }
    RunnerDecl { name, model, system, tools, temperature, max_tokens }
}

fn parse_stage(pair: Pair<Rule>) -> StageDecl {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut runner: Option<String> = None;
    let mut prompt = None;
    let mut runs = Vec::new();

    for field in inner {
        let mut fi = field.into_inner();
        let sub = fi.next().unwrap();
        match sub.as_rule() {
            Rule::stage_in => {
                inputs = sub.into_inner().map(parse_artifact_decl).collect();
            }
            Rule::stage_out => {
                outputs = sub.into_inner().map(parse_artifact_decl).collect();
            }
            Rule::stage_runner => {
                runner = Some(sub.into_inner().next().unwrap().as_str().to_string());
            }
            Rule::stage_prompt => {
                let pv = sub.into_inner().next().unwrap();
                prompt = Some(parse_prompt_val(pv));
            }
            Rule::run_decl => {
                runs.push(parse_run_decl(sub));
            }
            _ => {}
        }
    }
    StageDecl { name, inputs, outputs, runner, prompt, runs }
}

fn parse_run_decl(pair: Pair<Rule>) -> RunDecl {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut runner = None;
    let mut prompt = None;
    let mut outputs = Vec::new();

    for field in inner {
        let mut fi = field.into_inner();
        let sub = fi.next().unwrap();
        match sub.as_rule() {
            Rule::run_runner => {
                runner = Some(sub.into_inner().next().unwrap().as_str().to_string());
            }
            Rule::run_prompt => {
                let pv = sub.into_inner().next().unwrap();
                prompt = Some(parse_prompt_val(pv));
            }
            Rule::run_out => {
                outputs = sub.into_inner().map(parse_artifact_decl).collect();
            }
            _ => {}
        }
    }
    RunDecl { name, runner, prompt, outputs }
}

fn parse_artifact_decl(pair: Pair<Rule>) -> ArtifactDecl {
    let mut inner = pair.into_inner().peekable();

    // First token: identifier (the artifact name)
    let name_tok = inner.next().unwrap().as_str();
    let name = name_tok.to_string();

    // Next might be the opt_marker rule or directly artifact_kind
    let optional = if inner.peek().map(|p| p.as_rule()) == Some(Rule::opt_marker) {
        inner.next(); // consume opt_marker
        true
    } else {
        false
    };

    // Next: artifact_kind
    let kind_pair = inner.next().unwrap();
    let kind = match kind_pair.as_str() {
        "file" => ArtifactKind::File,
        _      => ArtifactKind::Ref,
    };

    // Optional: seed_init
    let seed_path = inner.next().map(|seed| {
        // seed_init contains a quoted_str
        unquote(seed.into_inner().next().unwrap().as_str())
    });

    ArtifactDecl { name, optional, kind, seed_path }
}

fn parse_pipeline(pair: Pair<Rule>) -> PipelineDecl {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();

    // optional inputs block, then pipe_start, then routes_block
    let next = inner.next().unwrap();
    let (inputs, start_pair) = if next.as_rule() == Rule::pipeline_inputs {
        (parse_pipeline_inputs(next), inner.next().unwrap())
    } else {
        (vec![], next)
    };
    let start = start_pair.into_inner().next().unwrap().as_str().to_string();

    let routes_pair = inner.next().unwrap();
    let routes = routes_pair.into_inner().map(parse_route).collect();

    PipelineDecl { name, inputs, start, routes }
}

fn parse_pipeline_inputs(pair: Pair<Rule>) -> Vec<InputDecl> {
    pair.into_inner().map(|p| {
        let mut inner = p.into_inner().peekable();
        let name = inner.next().unwrap().as_str().to_string();
        let optional = if inner.peek().map(|p| p.as_rule()) == Some(Rule::opt_marker) {
            inner.next();
            true
        } else {
            false
        };
        let kind = match inner.next().unwrap().as_str() {
            "file" => ArtifactKind::File,
            _      => ArtifactKind::Ref,
        };
        InputDecl { name, optional, kind }
    }).collect()
}

fn parse_route(pair: Pair<Rule>) -> Route {
    let mut inner = pair.into_inner();

    // route_source is silent (_), so we get predicate/fan_in_src/stage_src directly
    let source_pair = inner.next().unwrap();
    let source = match source_pair.as_rule() {
        Rule::predicate => {
            let mut pi = source_pair.into_inner();
            let stage    = pi.next().unwrap().as_str().to_string();
            let artifact = pi.next().unwrap().as_str().to_string();
            let op = match pi.next().unwrap().as_str() {
                "==" => CompareOp::Eq,
                _    => CompareOp::Ne,
            };
            let value = unquote(pi.next().unwrap().as_str());
            RouteSource::Predicate { stage, artifact, op, value }
        }
        Rule::fan_in_src => {
            let stage = source_pair.into_inner().next().unwrap().as_str().to_string();
            RouteSource::FanIn(stage)
        }
        Rule::stage_src => {
            let stage = source_pair.into_inner().next().unwrap().as_str().to_string();
            RouteSource::Stage(stage)
        }
        _ => RouteSource::Stage(source_pair.as_str().to_string()),
    };

    // stage_target: identifier fan_out_spec?
    let target_pair = inner.next().unwrap();
    let mut ti = target_pair.into_inner();
    let stage = ti.next().unwrap().as_str().to_string();
    let parallel_spec = ti.next().map(|spec| {
        // fan_out_spec inner: optional pos_int
        let limit = spec.into_inner().next().map(|n| n.as_str().parse::<u32>().unwrap());
        ParallelSpec { limit }
    });

    // Optional parallel_kw rule
    let parallel = inner.next().map(|p| p.as_rule() == Rule::parallel_kw).unwrap_or(false);

    Route {
        source,
        target: RouteTarget { stage, parallel_spec },
        parallel,
    }
}

fn parse_prompt_val(pair: Pair<Rule>) -> PromptSource {
    // pair is prompt_val, inner is file_ref | quoted_str
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::file_ref => {
            let path = unquote(inner.into_inner().next().unwrap().as_str());
            PromptSource::File(path)
        }
        _ => PromptSource::Inline(unquote(inner.as_str())),
    }
}

fn parse_string_val(pair: Pair<Rule>) -> String {
    // pair is string_val, inner is quoted_str | model_id
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::quoted_str => unquote(inner.as_str()),
        _                => inner.as_str().to_string(),
    }
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_import() {
        let items = parse_str(r#"import "stages/workers.line""#).unwrap();
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], TlItem::Import(p) if p == "stages/workers.line"));
    }

    #[test]
    fn test_parse_runner_inline_system() {
        let src = r#"
runner my-runner {
  model: claude-sonnet-4-6
  system: "You are helpful."
}
"#;
        let items = parse_str(src).unwrap();
        assert_eq!(items.len(), 1);
        let TlItem::Runner(r) = &items[0] else { panic!("expected runner") };
        assert_eq!(r.name, "my-runner");
        assert_eq!(r.model, "claude-sonnet-4-6");
        assert!(matches!(&r.system, Some(PromptSource::Inline(s)) if s == "You are helpful."));
    }

    #[test]
    fn test_parse_runner_file_system() {
        let src = r#"
runner eng-lead {
  model: claude-opus-4-8
  system: file("prompts/eng-lead.md")
  tools: [read_file, write_file]
  temperature: 0.7
  max_tokens: 4096
}
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Runner(r) = &items[0] else { panic!() };
        assert!(matches!(&r.system, Some(PromptSource::File(p)) if p == "prompts/eng-lead.md"));
        assert_eq!(r.tools, vec!["read_file", "write_file"]);
        assert_eq!(r.temperature, Some(0.7));
        assert_eq!(r.max_tokens, Some(4096));
    }

    #[test]
    fn test_parse_stage_basic() {
        let src = r#"
stage interview {
  in: brief? as file("specs/brief.md")
  out: spec as file
       verdict as ref
  runner: feature-interviewer
  prompt: file("prompts/task.md")
}
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Stage(s) = &items[0] else { panic!() };
        assert_eq!(s.name, "interview");
        assert_eq!(s.inputs.len(), 1);
        assert!(s.inputs[0].optional);
        assert_eq!(s.inputs[0].name, "brief");
        assert_eq!(s.inputs[0].kind, ArtifactKind::File);
        assert_eq!(s.inputs[0].seed_path, Some("specs/brief.md".to_string()));
        assert_eq!(s.outputs.len(), 2);
        assert_eq!(s.outputs[0].name, "spec");
        assert_eq!(s.outputs[0].kind, ArtifactKind::File);
        assert_eq!(s.outputs[1].name, "verdict");
        assert_eq!(s.outputs[1].kind, ArtifactKind::Ref);
        assert_eq!(s.runner, Some("feature-interviewer".to_string()));
        assert!(matches!(&s.prompt, Some(PromptSource::File(p)) if p == "prompts/task.md"));
    }

    #[test]
    fn test_parse_pipeline_routes() {
        let src = r#"
pipeline feature-dev {
  start: interview
  routes {
    interview.verdict == "approved" -> review
    interview.verdict == "rejected" -> interview
    review -> tip
    tip -> implement[*3] parallel
    implement[*] -> summary
  }
}
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Pipeline(p) = &items[0] else { panic!() };
        assert_eq!(p.name, "feature-dev");
        assert_eq!(p.start, "interview");
        assert_eq!(p.routes.len(), 5);

        // Predicate route: approved
        assert!(matches!(&p.routes[0].source,
            RouteSource::Predicate { stage, artifact, op, value }
            if stage == "interview" && artifact == "verdict"
               && *op == CompareOp::Eq && value == "approved"));

        // Predicate route: rejected (retry loop)
        assert!(matches!(&p.routes[1].source,
            RouteSource::Predicate { value, .. } if value == "rejected"));
        assert_eq!(p.routes[1].target.stage, "interview");

        // Fan-out
        assert!(p.routes[3].parallel);
        assert_eq!(p.routes[3].target.stage, "implement");
        assert_eq!(p.routes[3].target.parallel_spec.as_ref().unwrap().limit, Some(3));

        // Fan-in
        assert!(matches!(&p.routes[4].source, RouteSource::FanIn(s) if s == "implement"));
        assert_eq!(p.routes[4].target.stage, "summary");
    }

    #[test]
    fn test_parse_comment() {
        let src = r#"
// this is a comment
runner r {
  model: claude-sonnet-4-6
  system: "s"
}
// another comment
"#;
        let items = parse_str(src).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_parse_multiple_items() {
        let src = r#"
import "other.line"
runner r { model: claude-sonnet-4-6 system: "s" }
stage a { runner: r }
pipeline p { start: a routes {} }
"#;
        let items = parse_str(src).unwrap();
        assert_eq!(items.len(), 4);
        assert!(matches!(&items[0], TlItem::Import(_)));
        assert!(matches!(&items[1], TlItem::Runner(_)));
        assert!(matches!(&items[2], TlItem::Stage(_)));
        assert!(matches!(&items[3], TlItem::Pipeline(_)));
    }

    #[test]
    fn test_parse_route_ne_predicate() {
        let src = r#"
pipeline p {
  start: a
  routes {
    a.verdict != "rejected" -> b
  }
}
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Pipeline(p) = &items[0] else { panic!() };
        assert!(matches!(&p.routes[0].source,
            RouteSource::Predicate { op, value, .. }
            if *op == CompareOp::Ne && value == "rejected"));
    }

    #[test]
    fn test_parse_config_block() {
        let src = r#"
config {
  model: claude-sonnet-4-6
}
"#;
        let items = parse_str(src).unwrap();
        assert_eq!(items.len(), 1);
        let TlItem::Config(c) = &items[0] else { panic!("expected config") };
        assert_eq!(c.model, Some("claude-sonnet-4-6".to_string()));
    }

    #[test]
    fn test_parse_empty_config_block() {
        let src = r#"config {}"#;
        let items = parse_str(src).unwrap();
        let TlItem::Config(c) = &items[0] else { panic!() };
        assert_eq!(c.model, None);
    }

    #[test]
    fn test_parse_stage_with_run_blocks() {
        let src = r#"
stage review {
  runner: analyst
  run fast {
    prompt: "Quick check."
    out: quick-verdict as ref
  }
  run thorough {
    runner: critic
    out: detailed-verdict as ref
  }
}
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Stage(s) = &items[0] else { panic!() };
        assert_eq!(s.name, "review");
        assert_eq!(s.runner, Some("analyst".to_string()));
        assert_eq!(s.runs.len(), 2);

        let fast = &s.runs[0];
        assert_eq!(fast.name, "fast");
        assert_eq!(fast.runner, None);
        assert!(matches!(&fast.prompt, Some(PromptSource::Inline(p)) if p == "Quick check."));
        assert_eq!(fast.outputs.len(), 1);
        assert_eq!(fast.outputs[0].name, "quick-verdict");

        let thorough = &s.runs[1];
        assert_eq!(thorough.name, "thorough");
        assert_eq!(thorough.runner, Some("critic".to_string()));
        assert_eq!(thorough.outputs[0].name, "detailed-verdict");
    }

    #[test]
    fn test_parse_pipeline_with_inputs() {
        let src = r#"
pipeline code-review {
  inputs {
    code     as file
    language as ref
    context? as ref
  }
  start: assess
  routes {
    assess -> report
  }
}
stage assess { runner: r }
stage report  { runner: r }
runner r { model: claude-sonnet-4-6 }
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Pipeline(p) = &items[0] else { panic!() };
        assert_eq!(p.inputs.len(), 3);
        assert_eq!(p.inputs[0].name, "code");
        assert!(!p.inputs[0].optional);
        assert_eq!(p.inputs[0].kind, ArtifactKind::File);
        assert_eq!(p.inputs[1].name, "language");
        assert!(!p.inputs[1].optional);
        assert_eq!(p.inputs[1].kind, ArtifactKind::Ref);
        assert_eq!(p.inputs[2].name, "context");
        assert!(p.inputs[2].optional);
    }

    #[test]
    fn test_parse_pipeline_without_inputs_still_valid() {
        let src = r#"
pipeline p {
  start: a
  routes {}
}
stage a {}
"#;
        let items = parse_str(src).unwrap();
        let TlItem::Pipeline(p) = &items[0] else { panic!() };
        assert!(p.inputs.is_empty());
    }
}
