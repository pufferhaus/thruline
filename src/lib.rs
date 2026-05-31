//! Thruline — deterministic agent pipeline runtime.
//!
//! Embed thruline in a Rust application:
//!
//! ```rust,no_run
//! use thruline::Runtime;
//! use thruline::runtime::state::RunState;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let items = thruline::parser::parse_file("pipeline.line".as_ref())?;
//! let state = RunState::new("run-1".into(), "my-pipeline".into(), "pipeline.line".into());
//! let mut runtime = Runtime::new(state, items);
//! # Ok(())
//! # }
//! ```

pub mod ast;
pub mod driver;
pub mod events;
pub mod parser;
pub mod runtime;
pub mod validator;

pub use runtime::Runtime;
pub use runtime::AdvanceOutcome;
pub use runtime::state::RunState;
pub use runtime::artifact::ArtifactStore;
pub use events::ThrulineEvent;
