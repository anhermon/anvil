use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// A row to be persisted in the `evolution_log` table.
pub struct EvolutionEntry<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub prompt_score: f64,
    pub outcome_kind: &'a str,
    pub outcome_detail: &'a str,
    pub created_at: &'a str,
}

/// A row retrieved from the `evolution_log` table.
pub struct EvolutionRecord {
    pub id: Uuid,
    pub session_id: Uuid,
    pub prompt_score: f64,
    pub outcome_kind: String,
    pub outcome_detail: String,
    pub created_at: DateTime<Utc>,
}

/// Insert a single evolution record into the database.
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

/// Query recent evolution log records.
pub async fn query_evolution_log(pool: &SqlitePool, limit: i64) -> Result<Vec<EvolutionRecord>> {
    let rows = sqlx::query(
        r#"SELECT id, session_id, prompt_score, outcome_kind, outcome_detail, created_at
           FROM evolution_log
           ORDER BY created_at DESC
           LIMIT ?"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::new();
    for row in rows {
        let id_str: String = row.try_get("id")?;
        let session_id_str: String = row.try_get("session_id")?;
        let created_at_str: String = row.try_get("created_at")?;

        results.push(EvolutionRecord {
            id: Uuid::parse_str(&id_str)?,
            session_id: Uuid::parse_str(&session_id_str)?,
            prompt_score: row.try_get("prompt_score")?,
            outcome_kind: row.try_get("outcome_kind")?,
            outcome_detail: row.try_get("outcome_detail")?,
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        });
    }
    Ok(results)
}
