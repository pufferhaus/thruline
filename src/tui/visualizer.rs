use crate::ast::{TlItem, PipelineDecl, StageDecl, Route, RouteSource, RouteTarget, ArtifactKind, ArtifactDecl, CompareOp};

#[derive(Debug, Clone)]
pub struct Graph {
    pub stages: Vec<String>,
    pub edges: Vec<Edge>,
    pub start: String,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,       // empty string for unconditional
    pub max_visits: Option<u32>,
    pub is_back_edge: bool,  // true if this edge creates a cycle
}

pub fn build_graph(
    items: &[TlItem],
    pipeline_name: Option<&str>,
) -> anyhow::Result<Graph> {
    let pipeline = items.iter()
        .filter_map(|i| if let TlItem::Pipeline(p) = i { Some(p) } else { None })
        .find(|p| pipeline_name.map_or(true, |name| p.name == name))
        .ok_or_else(|| anyhow::anyhow!("no pipeline found"))?;

    // Collect all stage names in insertion order (start first, then route sources/targets)
    let mut stages: Vec<String> = Vec::new();
    let mut seen_stages = std::collections::HashSet::new();

    fn add_stage_fn(name: &str, stages: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
        if seen.insert(name.to_string()) {
            stages.push(name.to_string());
        }
    }
    add_stage_fn(&pipeline.start, &mut stages, &mut seen_stages);
    for route in &pipeline.routes {
        add_stage_fn(source_stage(&route.source), &mut stages, &mut seen_stages);
        add_stage_fn(&route.target.stage, &mut stages, &mut seen_stages);
    }

    // Detect back edges via DFS from start
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut in_stack: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut back_edges: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    fn dfs(
        node: &str,
        pipeline: &PipelineDecl,
        visited: &mut std::collections::HashSet<String>,
        in_stack: &mut std::collections::HashSet<String>,
        back_edges: &mut std::collections::HashSet<(String, String)>,
    ) {
        if in_stack.contains(node) { return; }
        if visited.contains(node) { return; }
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());
        for route in &pipeline.routes {
            if source_stage(&route.source) == node {
                let to = &route.target.stage;
                if in_stack.contains(to.as_str()) {
                    back_edges.insert((node.to_string(), to.clone()));
                } else {
                    dfs(to, pipeline, visited, in_stack, back_edges);
                }
            }
        }
        in_stack.remove(node);
    }

    dfs(&pipeline.start, pipeline, &mut visited, &mut in_stack, &mut back_edges);

    let edges = pipeline.routes.iter().map(|route| {
        let from = source_stage(&route.source).to_string();
        let to = route.target.stage.clone();
        let is_back_edge = back_edges.contains(&(from.clone(), to.clone()));
        Edge {
            from: from.clone(),
            to,
            label: edge_label(&route.source),
            max_visits: route.max_visits,
            is_back_edge,
        }
    }).collect();

    Ok(Graph { stages, edges, start: pipeline.start.clone() })
}

pub fn render_graph(graph: &Graph) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rendered: std::collections::HashSet<String> = std::collections::HashSet::new();
    render_node(&graph.start, graph, &mut lines, &mut rendered, 0);
    lines
}

fn render_node(
    stage: &str,
    graph: &Graph,
    lines: &mut Vec<String>,
    rendered: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    let indent = "  ".repeat(depth);

    if rendered.contains(stage) {
        lines.push(format!("{}[{}] (already rendered above)", indent, stage));
        return;
    }
    rendered.insert(stage.to_string());

    lines.push(format!("{}[{}]", indent, stage));

    let outgoing: Vec<&Edge> = graph.edges.iter()
        .filter(|e| e.from == stage)
        .collect();

    if outgoing.is_empty() { return; }

    if outgoing.len() == 1 {
        let e = outgoing[0];
        if e.is_back_edge {
            let max_str = e.max_visits.map(|n| format!(" [max:{}]", n)).unwrap_or_default();
            lines.push(format!("{}  │", indent));
            lines.push(format!("{}  └─→ [{}] (loop ↑){}", indent, e.to, max_str));
        } else {
            if !e.label.is_empty() {
                let max_str = e.max_visits.map(|n| format!(" [max:{}]", n)).unwrap_or_default();
                lines.push(format!("{}  │ {}{}", indent, e.label, max_str));
            } else {
                lines.push(format!("{}  │", indent));
            }
            render_node(&e.to, graph, lines, rendered, depth);
        }
    } else {
        // Branch: multiple outgoing edges
        lines.push(format!("{}  │", indent));
        lines.push(format!("{}  ├── branches ──", indent));
        for (i, e) in outgoing.iter().enumerate() {
            let connector = if i + 1 == outgoing.len() { "└" } else { "├" };
            if e.is_back_edge {
                let max_str = e.max_visits.map(|n| format!(" [max:{}]", n)).unwrap_or_default();
                let label = if e.label.is_empty() { String::new() } else { format!(" ({})", e.label) };
                lines.push(format!("{}  {}─→ [{}] (loop ↑){}{}", indent, connector, e.to, label, max_str));
            } else {
                let label = if e.label.is_empty() { String::new() } else { format!(" ({})", e.label) };
                lines.push(format!("{}  {}─ {}", indent, connector, label.trim()));
                render_node(&e.to, graph, lines, rendered, depth + 2);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn edge_label(source: &RouteSource) -> String {
    match source {
        RouteSource::Stage(_) | RouteSource::FanIn(_) => String::new(),
        RouteSource::Predicate { artifact, op, value, .. } => {
            let op_str = match op { CompareOp::Eq => "==", CompareOp::Ne => "!=" };
            format!("{} {} \"{}\"", artifact, op_str, value)
        }
    }
}

fn source_stage(source: &RouteSource) -> &str {
    match source {
        RouteSource::Stage(s) | RouteSource::FanIn(s) => s,
        RouteSource::Predicate { stage, .. } => stage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{InputDecl, PromptSource};

    fn make_items(pipeline: PipelineDecl, stages: Vec<StageDecl>) -> Vec<TlItem> {
        let mut items: Vec<TlItem> = stages.into_iter().map(TlItem::Stage).collect();
        items.push(TlItem::Pipeline(pipeline));
        items
    }

    fn simple_stage(name: &str) -> StageDecl {
        StageDecl {
            name: name.to_string(),
            inputs: vec![],
            outputs: vec![],
            runner: None,
            prompt: None,
            runs: vec![],
        }
    }

    fn unconditional_route(from: &str, to: &str) -> Route {
        Route {
            source: RouteSource::Stage(from.to_string()),
            target: RouteTarget { stage: to.to_string(), parallel_spec: None },
            max_visits: None,
        }
    }

    fn predicate_route(stage: &str, artifact: &str, op: CompareOp, value: &str, to: &str) -> Route {
        Route {
            source: RouteSource::Predicate {
                stage: stage.to_string(),
                artifact: artifact.to_string(),
                op,
                value: value.to_string(),
            },
            target: RouteTarget { stage: to.to_string(), parallel_spec: None },
            max_visits: None,
        }
    }

    #[test]
    fn test_build_graph_linear() {
        let p = PipelineDecl {
            name: "p".to_string(),
            inputs: vec![],
            start: "a".to_string(),
            routes: vec![unconditional_route("a", "b")],
        };
        let items = make_items(p, vec![simple_stage("a"), simple_stage("b")]);
        let graph = build_graph(&items, None).unwrap();
        assert_eq!(graph.start, "a");
        assert!(graph.stages.contains(&"a".to_string()));
        assert!(graph.stages.contains(&"b".to_string()));
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, "a");
        assert_eq!(graph.edges[0].to, "b");
        assert!(!graph.edges[0].is_back_edge);
    }

    #[test]
    fn test_build_graph_loop_marked() {
        // a -> b -> a (loop)
        let p = PipelineDecl {
            name: "p".to_string(),
            inputs: vec![],
            start: "a".to_string(),
            routes: vec![
                unconditional_route("a", "b"),
                unconditional_route("b", "a"),
            ],
        };
        let items = make_items(p, vec![simple_stage("a"), simple_stage("b")]);
        let graph = build_graph(&items, None).unwrap();
        let back = graph.edges.iter().find(|e| e.to == "a").unwrap();
        assert!(back.is_back_edge, "b->a should be a back edge");
    }

    #[test]
    fn test_build_graph_predicate_label() {
        let p = PipelineDecl {
            name: "p".to_string(),
            inputs: vec![],
            start: "review".to_string(),
            routes: vec![
                predicate_route("review", "verdict", CompareOp::Eq, "approved", "done"),
                predicate_route("review", "verdict", CompareOp::Ne, "approved", "revise"),
            ],
        };
        let items = make_items(p, vec![simple_stage("review"), simple_stage("done"), simple_stage("revise")]);
        let graph = build_graph(&items, None).unwrap();
        let approved_edge = graph.edges.iter().find(|e| e.to == "done").unwrap();
        assert_eq!(approved_edge.label, r#"verdict == "approved""#);
    }

    #[test]
    fn test_build_graph_selects_by_name() {
        let p1 = PipelineDecl { name: "first".to_string(), inputs: vec![], start: "a".to_string(), routes: vec![] };
        let p2 = PipelineDecl { name: "second".to_string(), inputs: vec![], start: "b".to_string(), routes: vec![] };
        let items = vec![
            TlItem::Stage(simple_stage("a")),
            TlItem::Stage(simple_stage("b")),
            TlItem::Pipeline(p1),
            TlItem::Pipeline(p2),
        ];
        let g = build_graph(&items, Some("second")).unwrap();
        assert_eq!(g.start, "b");
    }

    #[test]
    fn test_build_graph_error_no_pipeline() {
        let items = vec![TlItem::Stage(simple_stage("a"))];
        assert!(build_graph(&items, None).is_err());
    }

    #[test]
    fn test_render_single_stage() {
        let graph = Graph {
            start: "a".to_string(),
            stages: vec!["a".to_string()],
            edges: vec![],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("[a]"), "got: {}", joined);
    }

    #[test]
    fn test_render_linear_two_stages() {
        let graph = Graph {
            start: "a".to_string(),
            stages: vec!["a".to_string(), "b".to_string()],
            edges: vec![Edge { from: "a".to_string(), to: "b".to_string(), label: String::new(), max_visits: None, is_back_edge: false }],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("[a]"), "got: {}", joined);
        assert!(joined.contains("[b]"), "got: {}", joined);
        // b appears after a
        let a_pos = joined.find("[a]").unwrap();
        let b_pos = joined.find("[b]").unwrap();
        assert!(b_pos > a_pos, "b should appear below a");
    }

    #[test]
    fn test_render_loop_shows_loop_marker() {
        let graph = Graph {
            start: "review".to_string(),
            stages: vec!["review".to_string(), "revise".to_string()],
            edges: vec![
                Edge { from: "review".to_string(), to: "revise".to_string(), label: r#"verdict != "approved""#.to_string(), max_visits: Some(4), is_back_edge: false },
                Edge { from: "revise".to_string(), to: "review".to_string(), label: String::new(), max_visits: None, is_back_edge: true },
            ],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("loop"), "expected loop marker, got: {}", joined);
        assert!(joined.contains("[max:4]"), "expected max:4, got: {}", joined);
    }

    #[test]
    fn test_render_branch_includes_both_targets() {
        let graph = Graph {
            start: "classify".to_string(),
            stages: vec!["classify".to_string(), "approve".to_string(), "reject".to_string()],
            edges: vec![
                Edge { from: "classify".to_string(), to: "approve".to_string(), label: r#"verdict == "ok""#.to_string(), max_visits: None, is_back_edge: false },
                Edge { from: "classify".to_string(), to: "reject".to_string(), label: r#"verdict != "ok""#.to_string(), max_visits: None, is_back_edge: false },
            ],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("[approve]"), "got: {}", joined);
        assert!(joined.contains("[reject]"), "got: {}", joined);
    }
}
