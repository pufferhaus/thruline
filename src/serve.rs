use axum::{routing::get, Router, Json, extract::Path, response::Html};
use crate::runtime::state::{list_runs, RunState};

pub async fn run_server(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(handler_index))
        .route("/api/runs", get(handler_list_runs))
        .route("/api/runs/{id}", get(handler_get_run));

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("thruline serve: http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handler_index() -> Html<&'static str> {
    Html(include_str!("serve_index.html"))
}

async fn handler_list_runs() -> Json<Vec<serde_json::Value>> {
    let runs = list_runs().unwrap_or_default();
    Json(runs.into_iter().map(|r| serde_json::to_value(r).unwrap()).collect())
}

async fn handler_get_run(Path(id): Path<String>) -> Result<Json<RunState>, axum::http::StatusCode> {
    RunState::load(&id)
        .map(Json)
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)
}
