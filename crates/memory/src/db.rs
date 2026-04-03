use anyhow::Result;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;
use uuid::Uuid;
use crate::episode::{Episode, EpisodeKind};

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

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_episodes_session ON episodes(session_id)")
            .execute(pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_episodes_created ON episodes(created_at)")
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

        Ok(())
    }

    /// Insert a new episode.
    pub async fn insert(&self, ep: &Episode) -> Result<()> {
        let kind = match ep.kind {
            EpisodeKind::Turn => "turn",
            EpisodeKind::ToolCall => "tool_call",
            EpisodeKind::Fact => "fact",
            EpisodeKind::Summary => "summary",
        };
        let metadata = ep.metadata.as_ref().map(|m| m.to_string());

        sqlx::query(
            r#"INSERT INTO episodes (id, session_id, kind, role, content, metadata, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(ep.id.to_string())
        .bind(ep.session_id.to_string())
        .bind(kind)
        .bind(&ep.role)
        .bind(&ep.content)
        .bind(metadata)
        .bind(ep.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Retrieve recent episodes for a session, newest-first, limited to `limit`.
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

    /// Full-text search across episode content.
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<Episode>> {
        let rows = sqlx::query(
            r#"SELECT e.id, e.session_id, e.kind, e.role, e.content, e.metadata, e.created_at
               FROM episodes_fts fts
               JOIN episodes e ON e.id = fts.id
               WHERE episodes_fts MATCH ?
               ORDER BY rank LIMIT ?"#,
        )
        .bind(query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(parse_row).collect()
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
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
        metadata: metadata_str.as_deref().and_then(|s| serde_json::from_str(s).ok()),
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
}
