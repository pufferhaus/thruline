# Language Evolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Three language changes: (D6) drop hyphens from identifiers — breaking but needed before adoption grows; (D2) `[max:N]` route limit syntax as a per-route override for the B4 runtime limit; (D1) `in ["a","b"]` value constraints on outputs for runtime validation.

**Architecture:** All changes follow the same path: grammar.pest → ast.rs → parser/mod.rs → validator.rs → runtime → docs. D6 is purely a grammar restriction. D2 adds `max_visits: Option<u32>` to `Route`. D1 adds `value_constraint: Option<Vec<String>>` to `ArtifactDecl` and runtime enforcement in `resume_stage`.

**Tech Stack:** Rust, pest PEG grammar, existing infrastructure.

---

## Files

- Modify: `src/parser/grammar.pest`
- Modify: `src/ast.rs`
- Modify: `src/parser/mod.rs`
- Modify: `src/validator.rs` (D1: constraint validation)
- Modify: `src/runtime/mod.rs` (D1: enforcement; D2: per-route limit)
- Modify: `src/runtime/state.rs` (D2: route-level limit supersedes global)
- Modify: `examples/code-review/review.line` (D6: remove hyphens from pipeline name)
- Modify: `docs/LANGUAGE.md`
- Modify: `tests/integration.rs`

---

### Task 1: Drop hyphens from identifiers (D6 — BREAKING)

**Files:**
- Modify: `src/parser/grammar.pest`
- Modify: `src/parser/mod.rs` (update any test fixtures using hyphens)
- Modify: `tests/integration.rs`
- Modify: `examples/code-review/review.line`
- Modify: `docs/LANGUAGE.md`

- [ ] **Step 1: Change identifier rule in `src/parser/grammar.pest`**

Change:
```
identifier  = @{ ASCII_ALPHA ~ (ASCII_ALPHANUMERIC | "_" | "-")* }
```
to:
```
identifier  = @{ ASCII_ALPHA ~ (ASCII_ALPHANUMERIC | "_")* }
```

Keep `model_id` unchanged (model IDs like `claude-opus-4-8` still need hyphens):
```
model_id    = @{ ASCII_ALPHA ~ (ASCII_ALPHANUMERIC | "_" | "-" | ".")* }
```

- [ ] **Step 2: Verify grammar fails on hyphenated identifiers**

```bash
echo 'thruline code-review { start: a routes {} }' | cargo run -- validate /dev/stdin 2>&1 | head -5
```
Expected: parse error mentioning `-`.

- [ ] **Step 3: Fix the code-review example**

In `examples/code-review/review.line`, change `thruline code-review {` to `thruline code_review {`.

- [ ] **Step 4: Scan parser tests for hyphenated identifiers**

```bash
grep -n 'code-review\|feature-dev\|dual-review' src/parser/mod.rs tests/integration.rs
```

Update any found instances: replace `-` with `_` in all identifier positions.

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: all pass.

- [ ] **Step 6: Update LANGUAGE.md**

In the `## Comments` / identifier section, note: identifiers use `[A-Za-z][A-Za-z0-9_]*` — no hyphens. Model IDs (in `model:` fields) still allow hyphens.

Remove the Feature Gaps entry `**Identifier syntax allows \`-\`**`.

- [ ] **Step 7: Commit**

```bash
git add src/parser/grammar.pest src/parser/mod.rs tests/integration.rs \
        examples/code-review/review.line docs/LANGUAGE.md
git commit -m "feat!: drop hyphens from identifiers — use underscores (D6, BREAKING)"
```

---

### Task 2: Per-route visit limit syntax — `[max:N]` (D2)

**Files:**
- Modify: `src/parser/grammar.pest`
- Modify: `src/ast.rs`
- Modify: `src/parser/mod.rs`
- Modify: `src/runtime/mod.rs`
- Modify: `docs/LANGUAGE.md`

- [ ] **Step 1: Add `route_limit` rule to grammar**

In `src/parser/grammar.pest`, in the Routes section, add:

```
route_limit = { "[max:" ~ pos_int ~ "]" }
route       = { route_source ~ "->" ~ stage_target ~ route_limit? }
```

- [ ] **Step 2: Add `max_visits: Option<u32>` to `Route` in `src/ast.rs`**

```rust
pub struct Route {
    pub source: RouteSource,
    pub target: RouteTarget,
    pub max_visits: Option<u32>,
}
```

- [ ] **Step 3: Parse `route_limit` in `src/parser/mod.rs`**

In `parse_route()`, after parsing `stage_target`, check for `Rule::route_limit`:

```rust
fn parse_route(pair: Pair<Rule>) -> Route {
    let mut inner = pair.into_inner();
    let source = parse_route_source(inner.next().unwrap());
    let target = parse_route_target(inner.next().unwrap());
    let max_visits = inner.next().map(|p| {
        // route_limit = { "[max:" ~ pos_int ~ "]" }
        p.into_inner().next().unwrap().as_str().parse::<u32>().unwrap()
    });
    Route { source, target, max_visits }
}
```

- [ ] **Step 4: Write a failing parse test**

In `src/parser/mod.rs` tests:

```rust
#[test]
fn test_parse_route_with_max_visits() {
    let src = r#"
thruline p {
  start: a
  routes {
    a -> b [max:5]
  }
}
stage a {}
stage b {}
"#;
    let items = parse_str(src).unwrap();
    let TlItem::Pipeline(p) = &items[0] else { panic!() };
    assert_eq!(p.routes[0].max_visits, Some(5));
}
```

Run to confirm it fails, then implement Step 3, then re-run to confirm pass.

- [ ] **Step 5: Enforce per-route limit in `route_after_stage()` in `src/runtime/mod.rs`**

In `route_after_stage()`, replace the global `MAX_STAGE_VISITS` check with:

```rust
// Per-route limit takes precedence over global; global is 100.
const GLOBAL_MAX: u32 = 100;
let route_max = route.max_visits.unwrap_or(GLOBAL_MAX);
let visits = *self.state.visit_counts.get(&next_stage).unwrap_or(&0);
if visits >= route_max {
    anyhow::bail!(
        "stage '{}' has been visited {} times (limit: {}) — possible infinite loop.",
        next_stage, visits, route_max
    );
}
```

- [ ] **Step 6: Write a runtime test for per-route limit**

```rust
#[test]
fn test_per_route_max_visits_overrides_global() {
    let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
    let items = vec![
        mk_runner("runner"),
        mk_stage("a", "runner", &[]),
        TlItem::Pipeline(PipelineDecl {
            name: "p".into(), inputs: vec![], start: "a".into(),
            routes: vec![Route {
                source: RouteSource::Stage("a".into()),
                target: RouteTarget { stage: "a".into(), parallel_spec: None },
                max_visits: Some(3),
            }],
        }),
    ];
    let mut rt = Runtime::new(state, items);
    rt.state.visit_counts.insert("a".to_string(), 3);
    rt.state.status = RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
    let err = rt.resume_stage("a", None, vec![]).unwrap_err();
    assert!(err.to_string().contains("3"), "expected limit 3 in error: {}", err);
}
```

- [ ] **Step 7: Update existing Route literals in tests**

Every `Route { source: ..., target: ... }` in test code needs `max_visits: None` added. Search and fix:

```bash
grep -n 'Route {' src/runtime/mod.rs src/validator.rs src/parser/mod.rs | head -20
```

Add `, max_visits: None` to each `Route { source: ..., target: ... }` struct literal.

- [ ] **Step 8: Run tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: all pass.

- [ ] **Step 9: Update LANGUAGE.md routes section**

Add `[max:N]` to the route syntax table:

```
<source> -> <target>[<fan-out>][max:N]
```

And document: "`[max:N]` — optional per-route visit cap. Overrides the global default of 100."

- [ ] **Step 10: Commit**

```bash
git add src/parser/grammar.pest src/ast.rs src/parser/mod.rs \
        src/runtime/mod.rs docs/LANGUAGE.md
git commit -m "feat: [max:N] per-route visit cap; overrides global 100-visit limit (D2)"
```

---

### Task 3: Output value constraints — `in ["a","b"]` (D1)

**Files:**
- Modify: `src/parser/grammar.pest`
- Modify: `src/ast.rs`
- Modify: `src/parser/mod.rs`
- Modify: `src/runtime/mod.rs` (enforce in `resume_stage`)
- Modify: `docs/LANGUAGE.md`

- [ ] **Step 1: Add `value_constraint` grammar rule**

In `src/parser/grammar.pest`, in the Artifact declarations section:

```
value_constraint = { "in" ~ "[" ~ quoted_str ~ ("," ~ quoted_str)* ~ "]" }
artifact_decl    = { artifact_source? ~ identifier ~ opt_marker? ~ "as" ~ artifact_kind ~ value_constraint? ~ seed_init? }
```

- [ ] **Step 2: Add `value_constraint: Option<Vec<String>>` to `ArtifactDecl` in `src/ast.rs`**

```rust
pub struct ArtifactDecl {
    pub name: String,
    pub source: Option<String>,
    pub optional: bool,
    pub kind: ArtifactKind,
    pub seed_path: Option<String>,
    pub value_constraint: Option<Vec<String>>,
}
```

- [ ] **Step 3: Parse `value_constraint` in `parse_artifact_decl()` in `src/parser/mod.rs`**

In `parse_artifact_decl`, after parsing `kind`, check for `Rule::value_constraint`:

```rust
fn parse_artifact_decl(pair: Pair<Rule>) -> ArtifactDecl {
    let mut inner = pair.into_inner();
    // Parse existing fields: source?, name, opt_marker?, kind
    // ... (existing code)
    
    // Check for value_constraint and seed_init in remaining tokens
    let mut value_constraint = None;
    let mut seed_path = None;
    for p in inner {
        match p.as_rule() {
            Rule::value_constraint => {
                value_constraint = Some(
                    p.into_inner()
                        .map(|s| {
                            let raw = s.as_str();
                            raw[1..raw.len()-1].to_string() // strip quotes
                        })
                        .collect()
                );
            }
            Rule::seed_init => {
                let raw = p.into_inner().next().unwrap().as_str();
                seed_path = Some(raw[1..raw.len()-1].to_string());
            }
            _ => {}
        }
    }
    ArtifactDecl { name, source, optional, kind, seed_path, value_constraint }
}
```

- [ ] **Step 4: Write a failing parse test**

```rust
#[test]
fn test_parse_value_constraint_on_output() {
    let src = r#"
stage classify {
  out: sentiment as value in ["positive","negative","neutral"]
}
"#;
    let items = parse_str(src).unwrap();
    let TlItem::Stage(s) = &items[0] else { panic!() };
    let out = &s.outputs[0];
    assert_eq!(out.name, "sentiment");
    assert_eq!(
        out.value_constraint.as_ref().unwrap(),
        &vec!["positive".to_string(), "negative".to_string(), "neutral".to_string()]
    );
}
```

Run to fail, implement, run to pass.

- [ ] **Step 5: Enforce constraints in `resume_stage()` in `src/runtime/mod.rs`**

In `resume_stage()`, in the `AwaitingResume` path, after the `StageComplete` emit and before the artifact store writes, add constraint checking:

```rust
// Validate value constraints on outputs.
let stages = self.stages();
if let Some(stage_decl) = stages.get(stage_name) {
    for (name, value, _is_file) in &outputs {
        if let Some(decl) = stage_decl.outputs.iter().find(|o| &o.name == name) {
            if let Some(allowed) = &decl.value_constraint {
                if !allowed.contains(value) {
                    anyhow::bail!(
                        "stage '{}' output '{}' = {:?} is not in allowed values {:?}",
                        stage_name, name, value, allowed
                    );
                }
            }
        }
    }
}
```

- [ ] **Step 6: Write an enforcement test**

```rust
#[test]
fn test_value_constraint_rejects_invalid_output() {
    use crate::ast::ArtifactDecl;
    let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
    let items = vec![
        mk_runner("runner"),
        TlItem::Stage(StageDecl {
            name: "classify".into(),
            inputs: vec![],
            outputs: vec![ArtifactDecl {
                name: "sentiment".into(), source: None, optional: false,
                kind: ArtifactKind::Value, seed_path: None,
                value_constraint: Some(vec![
                    "positive".into(), "negative".into(), "neutral".into()
                ]),
            }],
            runner: Some("runner".into()),
            prompt: None, runs: vec![],
        }),
        TlItem::Pipeline(PipelineDecl {
            name: "p".into(), inputs: vec![], start: "classify".into(), routes: vec![],
        }),
    ];
    let mut rt = Runtime::new(state, items);
    rt.state.status = RunStatus::AwaitingResume { stage: "classify".into(), parallel: None };
    let err = rt.resume_stage("classify", None, vec![
        ("sentiment".into(), "VERY_POSITIVE".into(), false)
    ]).unwrap_err();
    assert!(err.to_string().contains("not in allowed values"), "got: {}", err);
}

#[test]
fn test_value_constraint_accepts_valid_output() {
    use crate::ast::ArtifactDecl;
    let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
    let items = vec![
        mk_runner("runner"),
        TlItem::Stage(StageDecl {
            name: "classify".into(), inputs: vec![],
            outputs: vec![ArtifactDecl {
                name: "sentiment".into(), source: None, optional: false,
                kind: ArtifactKind::Value, seed_path: None,
                value_constraint: Some(vec!["positive".into(), "negative".into()]),
            }],
            runner: Some("runner".into()), prompt: None, runs: vec![],
        }),
        TlItem::Pipeline(PipelineDecl {
            name: "p".into(), inputs: vec![], start: "classify".into(), routes: vec![],
        }),
    ];
    let mut rt = Runtime::new(state, items);
    rt.state.status = RunStatus::AwaitingResume { stage: "classify".into(), parallel: None };
    rt.resume_stage("classify", None, vec![
        ("sentiment".into(), "positive".into(), false)
    ]).unwrap();
    assert_eq!(rt.state.artifacts.get_ref("classify.sentiment"), Some("positive"));
}
```

- [ ] **Step 7: Fix all existing `ArtifactDecl` struct literals**

Search for all places `ArtifactDecl {` is constructed without `value_constraint`:

```bash
grep -n 'ArtifactDecl {' src/runtime/mod.rs src/validator.rs src/parser/mod.rs | head -30
```

Add `value_constraint: None` to each one.

- [ ] **Step 8: Run tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: all pass.

- [ ] **Step 9: Update LANGUAGE.md**

In the Artifact Declarations table, add:

| `in ["a","b",...]` | Optional value constraint — `resume` fails with `stage_error` if output is not in the list. Only applies to `value` artifacts. |

Add an example:
```
stage classify {
  out: sentiment as value in ["positive","negative","neutral"]
}
```

- [ ] **Step 10: Commit and push**

```bash
git add src/parser/grammar.pest src/ast.rs src/parser/mod.rs \
        src/runtime/mod.rs docs/LANGUAGE.md
git commit -m "feat: value constraints on outputs — in ['a','b'] rejects invalid model output (D1)"
git push origin main
```
