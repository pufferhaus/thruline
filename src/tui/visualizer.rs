pub struct Graph {
    pub stages: Vec<String>,
    pub edges: Vec<Edge>,
    pub start: String,
}

pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,
    pub max_visits: Option<u32>,
}

pub fn build_graph(
    _items: &[crate::ast::TlItem],
    _pipeline_name: Option<&str>,
) -> anyhow::Result<Graph> {
    todo!()
}

pub fn render_graph(_graph: &Graph) -> Vec<String> {
    todo!()
}
