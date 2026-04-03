# AGENTS.md — crates/tools

This is the `harness-tools` library crate. It owns the tool registry, JSON Schema validation, and
all built-in tool implementations. Read this file before adding or modifying any tool.

---

## Responsibilities

| Module         | What it owns                                                             |
|----------------|--------------------------------------------------------------------------|
| `registry.rs`  | `ToolRegistry`, `ToolHandler` trait, `ToolOutput`                        |
| `schema.rs`    | `ToolSchema`, `validate()` — JSON Schema input validation                |
| `builtin.rs`   | Built-in tool implementations (`EchoTool`, …)                            |

---

## Implementing a new tool

Every tool is a struct that implements `ToolHandler`:

```rust
#[async_trait]
pub trait ToolHandler: Send + Sync + 'static {
    fn schema(&self) -> ToolSchema;
    async fn call(&self, input: Value) -> ToolOutput;
}
```

Step-by-step:

1. Add the tool struct (and its impl) to `crates/tools/src/builtin.rs`, or create a new file
   under `crates/tools/src/` and `pub mod` it from `lib.rs`.
2. `schema()` must return a `ToolSchema` with a unique `name`, a clear `description`, and a
   complete `parameters` object following JSON Schema draft-07.
3. `call()` receives the already-validated `input: Value`. Do not re-validate inside `call()` —
   `ToolRegistry::call()` runs `schema.validate()` automatically before dispatch.
4. Return `ToolOutput::ok(content)` on success or `ToolOutput::err(message)` on failure. Do not
   panic.
5. Register the tool in `Agent::new()` in `crates/cli/src/agent.rs` so it is available at runtime.
6. Write at least one unit test (see Testing below).

---

## Input validation

`ToolRegistry::call()` calls `schema.validate(&input)` before invoking `handler.call(input)`.
If validation fails the registry returns a `ToolOutput::err(...)` immediately — the handler is
never called with invalid input.

This means:
- You do not need defensive `.get("field").ok_or(...)` checks for required fields.
- You do still need to handle optional fields correctly (they may be absent from the validated
  input).
- If your tool requires a field not declared in `schema().parameters`, add it to the schema.

---

## Security: file system sandboxing

Any tool that reads or writes files **must** restrict access to safe roots:

- `~/.paperclip/workspace/` (the harness workspace directory)
- The current working directory at process start

Enforcement pattern:
```rust
use std::path::PathBuf;

fn safe_path(requested: &str, workspace: &PathBuf) -> anyhow::Result<PathBuf> {
    let canonical = workspace.join(requested).canonicalize()?;
    if !canonical.starts_with(workspace) {
        anyhow::bail!("path escapes sandbox: {}", requested);
    }
    Ok(canonical)
}
```

Reject any input path that resolves outside the sandbox root after canonicalization. Return
`ToolOutput::err(...)` rather than panicking.

---

## Async and runtime constraints

- Do not import or start a tokio runtime inside a tool implementation. Tools are always called
  from within an existing tokio context provided by the agent loop in `crates/cli`.
- Blocking I/O (reading large files, spawning subprocesses) must use `tokio::task::spawn_blocking`.
- Tool execution should be bounded in time. Long-running tools should respect a timeout passed
  via their input schema.

---

## ToolOutput

```rust
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self { ... }
    pub fn err(msg: impl Into<String>) -> Self { ... }
}
```

- `content` is always a UTF-8 string. For structured data, serialize to JSON.
- Set `is_error = true` (via `ToolOutput::err`) for any failure condition. Do not return a
  success output with an error message embedded in the content string.

---

## Testing

- Write a `#[cfg(test)] mod tests { ... }` block at the bottom of each tool source file.
- Use `#[tokio::test]` for async tool tests.
- Do not touch the real file system. Use `tempdir` or restrict tests to in-memory data.
- Do not make network calls in tests.
- Test the unhappy path: invalid input (the registry will reject it before `call()`, but test
  `call()` directly with edge-case inputs to verify robustness), sandboxing rejections, etc.

Example test skeleton:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolRegistry;

    #[tokio::test]
    async fn my_tool_happy_path() {
        let registry = ToolRegistry::new();
        registry.register(MyTool);
        let out = registry.call("my_tool", serde_json::json!({"key": "value"})).await;
        assert!(!out.is_error);
        assert!(out.content.contains("expected"));
    }
}
```

---

## Constraints summary

| Rule | Detail |
|------|--------|
| No runtime import | Do not start a tokio runtime; receive context from caller |
| Schema completeness | All required fields must be declared in `schema().parameters` |
| No re-validation | Do not duplicate validation that `ToolRegistry::call()` already does |
| Sandbox enforcement | File tools must reject paths outside `~/.paperclip/workspace/` or CWD |
| `ToolOutput::err` for failures | Never panic; return `ToolOutput::err` for all error conditions |
| No network in tests | All tool tests must be offline |
