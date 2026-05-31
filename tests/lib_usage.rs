use thruline::{Runtime, RunState};
use thruline::ast::TlItem;

#[test]
fn test_runtime_constructable_from_lib() {
    let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
    let items: Vec<TlItem> = vec![];
    let runtime = Runtime::new(state, items);
    assert_eq!(runtime.state.run_id, "r");
    assert_eq!(runtime.state.pipeline, "p");
}
