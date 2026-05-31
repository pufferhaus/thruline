mod ast;
mod cli;
mod driver;
mod events;
mod lsp;
mod parser;
mod runtime;
mod validator;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}
