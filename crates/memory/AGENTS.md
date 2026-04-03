# AGENTS.md — crates/memory

This is the `harness-memory` library crate. It owns all persistent episodic memory: storage,
retrieval, and full-text search. Read this file before modifying anything related to the database
schema or episode handling.

---

## Responsibilities

| Module       | What it owns                                                              |
|--------------|---------------------------------------------------------------------------|
| `db.rs`      | `MemoryDb` — SQLite pool, schema bootstrap, `insert`, `recent`, `search` |
| `episode.rs` | `Episode`, `EpisodeKind` — the memory record type                         |
| `lib.rs`     | Re-exports: `MemoryDb`, `Episode`, `EpisodeKind`                          |

---

## Storage engine

- **SQLite** via `sqlx` with the `runtime-tokio` feature.
- **FTS5** virtual table (`episodes_fts`) for full-text search over episode content.
- **Semantic recall** via `sqlite-vec` is planned (Phase 3) but not yet implemented. Do not add
  it without a tracking issue.
- Database file location: `~/.paperclip/harness/memory.db` (configurable via `MemoryConfig.db_path`).

---

## Schema

The current schema is bootstrapped inline in `MemoryDb::run_migrations()`. It creates:

- `episodes` table — primary storage
- `idx_episodes_session` — index on `session_id`
- `idx_episodes_created` — index on `created_at`
- `episodes_fts` — FTS5 virtual table (content mirror of `episodes.content`)
- `episodes_ai` — AFTER INSERT trigger that keeps `episodes_fts` in sync
- `sessions` table — session-level records

### Migration policy

**Never modify an existing migration.** The schema bootstrap in `run_migrations()` uses
`CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`, which means it is safe to re-run
but will not apply changes to an existing table.

When the schema must change:
1. Add new `ALTER TABLE` or `CREATE TABLE` statements **after** all existing statements in
   `run_migrations()`.
2. Gate the new statement so it is idempotent (e.g. `CREATE INDEX IF NOT EXISTS`, or check
   `PRAGMA table_info` before altering a column).
3. Document the change with a comment in `run_migrations()` that includes the date and a brief
   reason.
4. Write a migration test that opens an in-memory database, runs migrations twice, and asserts
   no error (idempotency check).

---

## EpisodeKind

Use the correct kind for every memory write. The LLM reads episode kinds as context signals.

| Kind       | When to use                                              |
|------------|----------------------------------------------------------|
| `Turn`     | A conversation turn (user message or assistant response) |
| `ToolCall` | A tool invocation and its result                         |
| `Fact`     | A distilled fact extracted from conversation             |
| `Summary`  | A session-level or segment-level summary                 |

Constructor helpers:
- `Episode::turn(session_id, role, content)` — creates a `Turn` episode.
- For other kinds, construct `Episode { kind: EpisodeKind::ToolCall, ... }` directly.

---

## Metadata

The `metadata` field is `Option<serde_json::Value>`. Rules:
- Always store valid JSON. Never store raw strings in `metadata`.
- Keep metadata small. Large blobs belong in `content`.
- Common metadata patterns:
  - Tool calls: `{"tool": "name", "input": {...}, "output": "..."}`
  - Facts: `{"source": "session_id", "confidence": 0.9}`
  - Summaries: `{"turn_range": [0, 20], "tokens": 1200}`

---

## MemoryDb API

```rust
// Open a persistent database (creates parent directories if needed)
let db = MemoryDb::open(&config.memory.db_path).await?;

// In-memory database for tests (no file I/O)
let db = MemoryDb::in_memory().await?;

// Insert an episode
db.insert(&episode).await?;

// Retrieve recent episodes for a session, newest-first
let episodes = db.recent(session_id, 20).await?;

// Full-text search across all episode content
let results = db.search("rust async", 10).await?;
```

---

## Testing

**Rule: use `MemoryDb::in_memory()` in every test. Never open a file-backed database in tests.**
File-backed databases create state that persists across test runs, interfere with parallel tests,
and leave behind temp files on CI.

```rust
#[tokio::test]
async fn insert_and_search() {
    let db = MemoryDb::in_memory().await.unwrap();
    let session_id = Uuid::new_v4();
    let ep = Episode::turn(session_id, "user", "the quick brown fox");
    db.insert(&ep).await.unwrap();

    let results = db.search("quick brown", 5).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "the quick brown fox");
}
```

Additional test requirements:
- Test each `EpisodeKind` round-trips correctly through insert/retrieve.
- Test that `metadata` serializes and deserializes as valid JSON.
- Test idempotency of `run_migrations()` (call it twice, expect no error).
- Use `#[tokio::test]` for all async tests.
- Do not use `unwrap()` in test assertions where a more descriptive assert is possible.

---

## Constraints summary

| Rule | Detail |
|------|--------|
| In-memory DB in tests | `MemoryDb::in_memory()` only — no file-backed databases in tests |
| No migration modification | Never change existing statements in `run_migrations()` — only append |
| Idempotent migrations | New schema changes must use `IF NOT EXISTS` or equivalent guards |
| Valid JSON metadata | `metadata` must always be a valid `serde_json::Value`, never a raw string |
| Correct EpisodeKind | Choose `Turn / ToolCall / Fact / Summary` based on the semantic content |
| No sqlite-vec yet | Do not add semantic recall without a tracking issue and Phase 3 context |
