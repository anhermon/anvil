use anyhow::Result;
use sqlx::SqlitePool;

/// A row to be persisted in the `evolution_records` table.
pub struct EvolutionEntry<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub prompt_score: f64,
    pub outcome_kind: &'a str,
    pub outcome_detail: &'a str,
    pub created_at: &'a str,
}

/// Insert a single evolution record into the database.
pub async fn insert_evolution_entry(pool: &SqlitePool, entry: &EvolutionEntry<'_>) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO evolution_records
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
