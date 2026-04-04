use crate::episode::{Episode, EpisodeKind};
use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::path::Path;
use uuid::Uuid;

/// SQLite-backed memory store.
pub struct MemoryDb {
    pool: SqlitePool,
}

impl MemoryDb {
    /// Open (or create) a SQLite database at `path` and run migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await?;

        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    /// In-memory database for tests.
    pub async fn in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS episodes (
                id          TEXT PRIMARY KEY NOT NULL,
                session_id  TEXT NOT NULL,
                kind        TEXT NOT NULL,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                metadata    TEXT,
                created_at  TEXT NOT NULL
            )"#,
        )
        .execute(pool)
        .await?;

        // Add session_name column if it does not exist yet (idempotent migration).
        let _ = sqlx::query("ALTER TABLE episodes ADD COLUMN session_name TEXT")
            .execute(pool)
            .await;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(session_id)")
            .execute(pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_episodes_created ON episodes(created_at)")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_episodes_session_name ON episodes(session_name)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"CREATE VIRTUAL TABLE IF NOT EXISTS episodes_fts USING fts5(
                id UNINDEXED,
                content,
                content='episodes',
                content_rowid='rowid'
            )"#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"CREATE TRIGGER IF NOT EXISTS episodes_ai AFTER INSERT ON episodes BEGIN
                INSERT INTO episodes_fts(rowid, id, content) VALUES (new.rowid, new.id, new.content);
            END"#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY NOT NULL,
                goal        TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'running',
                started_at  TEXT NOT NULL,
                finished_at TEXT
            )"#,
        )
        .execute(pool)
        .await?;

        // Migration: add session_name column to episodes if it doesn't exist.
        // SQLite doesn't support IF NOT EXISTS on ALTER TABLE, so we check the
        // column list first.
        let cols: Vec<String> = sqlx::query("PRAGMA table_info(episodes)")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                let name: String = row.try_get("name").unwrap_or_default();
                name
            })
            .collect();

        if !cols.iter().any(|c| c == "session_name") {
            sqlx::query("ALTER TABLE episodes ADD COLUMN session_name TEXT")
                .execute(pool)
                .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_episodes_session_name ON episodes(session_name)",
            )
            .execute(pool)
            .await?;
        }

        // Evolution log — written by harness-evolution, never modified in-place.
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS evolution_log (
                id              TEXT PRIMARY KEY NOT NULL,
                session_id      TEXT NOT NULL,
                prompt_score    REAL NOT NULL,
                outcome_kind    TEXT NOT NULL,
                outcome_detail  TEXT NOT NULL,
                created_at      TEXT NOT NULL
            )"#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_evolution_session ON evolution_log(session_id)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Insert a new episode (no named session tag).
    pub async fn insert(&self, ep: &Episode) -> Result<()> {
        self.insert_named(ep, None).await
    }

    /// Insert a new episode, optionally tagged with a named session.
    pub async fn insert_named(&self, ep: &Episode, session_name: Option<&str>) -> Result<()> {
        let kind = match ep.kind {
            EpisodeKind::Turn => "turn",
            EpisodeKind::ToolCall => "tool_call",
            EpisodeKind::Fact => "fact",
            EpisodeKind::Summary => "summary",
        };
        let metadata = ep.metadata.as_ref().map(|m| m.to_string());

        sqlx::query(
            r#"INSERT INTO episodes (id, session_id, kind, role, content, metadata, created_at, session_name)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(ep.id.to_string())
        .bind(ep.session_id.to_string())
        .bind(kind)
        .bind(&ep.role)
        .bind(&ep.content)
        .bind(metadata)
        .bind(ep.created_at.to_rfc3339())
        .bind(session_name)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Retrieve recent episodes for a session UUID, newest-first, limited to `limit`.
    pub async fn recent(&self, session_id: Uuid, limit: i64) -> Result<Vec<Episode>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, kind, role, content, metadata, created_at
               FROM episodes WHERE session_id = ?
               ORDER BY created_at DESC LIMIT ?"#,
        )
        .bind(session_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(parse_row).collect()
    }

    /// Retrieve recent episodes for a named session, oldest-first (chronological
    /// order so they can be replayed as conversation history), limited to `limit`.
    pub async fn recent_by_name(&self, session_name: &str, limit: i64) -> Result<Vec<Episode>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, kind, role, content, metadata, created_at
               FROM episodes WHERE session_name = ?
               ORDER BY created_at ASC LIMIT ?"#,
        )
        .bind(session_name)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(parse_row).collect()
    }

    /// Full-text search across episode content.
    ///
    /// The query is split into individual terms and joined with FTS5 OR
    /// syntax so that a multi-word goal like "explain rust ownership" matches
    /// episodes that contain any of those words rather than requiring all of
    /// them to be present.
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<Episode>> {
        // Convert "explain rust ownership" -> "explain OR rust OR ownership"
        let fts_query = query.split_whitespace().collect::<Vec<_>>().join(" OR ");

        let rows = sqlx::query(
            r#"SELECT e.id, e.session_id, e.kind, e.role, e.content, e.metadata, e.created_at
               FROM episodes_fts fts
               JOIN episodes e ON e.id = fts.id
               WHERE episodes_fts MATCH ?
               ORDER BY rank LIMIT ?"#,
        )
        .bind(fts_query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(parse_row).collect()
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

// ---------------------------------------------------------------------------
// Evolution log helpers (used by harness-evolution without a circular dep)
// ---------------------------------------------------------------------------

/// A thin record mirroring `harness_evolution::types::EvolutionRecord`.
///
/// Defined here so harness-memory has no dependency on harness-evolution.
#[derive(Debug)]
pub struct EvolutionEntry<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub prompt_score: f64,
    pub outcome_kind: &'a str,
    pub outcome_detail: &'a str,
    pub created_at: &'a str,
}

/// Insert one evolution record into the `evolution_log` table.
///
/// Called by `harness-evolution` via the pool returned by [`MemoryDb::pool`].
pub async fn insert_evolution_entry(pool: &SqlitePool, entry: &EvolutionEntry<'_>) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO evolution_log
               (id, session_id, prompt_score, outcome_kind, outcome_detail, created_at)
               VALUES (?, ?, ?, ?, ?, ?)"#,
    )
    .bind(entry.id)
    .bind(entry.session_id)
    .bind(entry.prompt_score)
    .bind(entry.outcome_kind)
    .bind(entry.outcome_detail)
    .bind(entry.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_row(row: &sqlx::sqlite::SqliteRow) -> Result<Episode> {
    let kind_str: String = row.try_get("kind")?;
    let kind = match kind_str.as_str() {
        "tool_call" => EpisodeKind::ToolCall,
        "fact" => EpisodeKind::Fact,
        "summary" => EpisodeKind::Summary,
        _ => EpisodeKind::Turn,
    };
    let id_str: String = row.try_get("id")?;
    let session_id_str: String = row.try_get("session_id")?;
    let metadata_str: Option<String> = row.try_get("metadata")?;
    let created_at_str: String = row.try_get("created_at")?;

    Ok(Episode {
        id: Uuid::parse_str(&id_str)?,
        session_id: Uuid::parse_str(&session_id_str)?,
        kind,
        role: row.try_get("role")?,
        content: row.try_get("content")?,
        metadata: metadata_str
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_and_retrieve() {
        let db = MemoryDb::in_memory().await.unwrap();
        let session_id = Uuid::new_v4();
        let ep = crate::Episode::turn(session_id, "user", "hello world");
        db.insert(&ep).await.unwrap();

        let results = db.recent(session_id, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "hello world");
    }

    #[tokio::test]
    async fn insert_named_and_retrieve_by_name() {
        let db = MemoryDb::in_memory().await.unwrap();
        let session_id = Uuid::new_v4();
        let ep = crate::Episode::turn(session_id, "user", "named session content");
        db.insert_named(&ep, Some("myproject")).await.unwrap();

        // Should be retrievable by session name.
        let results = db.recent_by_name("myproject", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "named session content");

        // Should NOT appear under a different session name.
        let other = db.recent_by_name("otherproject", 10).await.unwrap();
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn recent_by_name_returns_chronological_order() {
        let db = MemoryDb::in_memory().await.unwrap();
        let session_id = Uuid::new_v4();

        for i in 0..3u32 {
            let mut ep = crate::Episode::turn(session_id, "user", format!("message {i}"));
            // Stagger timestamps so ordering is deterministic.
            ep.created_at = chrono::Utc::now() + chrono::Duration::milliseconds(i as i64 * 10);
            db.insert_named(&ep, Some("ordered-session")).await.unwrap();
        }

        let results = db.recent_by_name("ordered-session", 10).await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].content, "message 0");
        assert_eq!(results[2].content, "message 2");
    }
}
