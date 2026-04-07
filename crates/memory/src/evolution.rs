use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

/// A row to be persisted in the `evolution_log` table.
pub struct EvolutionEntry<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub prompt_score: f64,
    pub outcome_kind: &'a str,
    pub outcome_detail: &'a str,
    pub created_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    Global,
    Workdir,
}

impl ScopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workdir => "workdir",
        }
    }

    pub fn from_db(value: &str) -> Self {
        if value == "workdir" {
            Self::Workdir
        } else {
            Self::Global
        }
    }
}

#[derive(Debug, Clone)]
pub struct EvolutionScope {
    pub kind: ScopeKind,
    pub key: Option<String>,
}

impl EvolutionScope {
    pub fn global() -> Self {
        Self {
            kind: ScopeKind::Global,
            key: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptVersionInput<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub scope_kind: &'a str,
    pub scope_key: Option<&'a str>,
    pub base_prompt_hash: &'a str,
    pub candidate_prompt: &'a str,
    pub candidate_diff: &'a str,
    pub score_before: f64,
    pub score_after: Option<f64>,
    pub created_at: &'a str,
}

#[derive(Debug, Clone)]
pub struct PromptVersionRecord {
    pub id: String,
    pub session_id: String,
    pub scope_kind: ScopeKind,
    pub scope_key: Option<String>,
    pub base_prompt_hash: String,
    pub candidate_prompt: String,
    pub candidate_diff: String,
    pub score_before: f64,
    pub score_after: Option<f64>,
    pub active: bool,
    pub replaced_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ValidationVoteEntry {
    pub id: String,
    pub record_id: String,
    pub candidate_id: String,
    pub validator: String,
    pub vote_kind: String,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ValidationVoteRecord {
    pub id: String,
    pub record_id: String,
    pub candidate_id: String,
    pub validator: String,
    pub vote_kind: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct EvolutionLogRecord {
    pub id: String,
    pub session_id: String,
    pub prompt_score: f64,
    pub outcome_kind: String,
    pub outcome_detail: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct EffectiveOverlay {
    pub selected_scope: ScopeKind,
    pub selected_scope_key: Option<String>,
    pub fallback_used: bool,
    pub version: PromptVersionRecord,
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

pub async fn insert_validation_votes(
    pool: &SqlitePool,
    votes: &[ValidationVoteEntry],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for vote in votes {
        sqlx::query(
            r#"INSERT INTO evolution_validation_votes
                   (id, record_id, candidate_id, validator, vote_kind, reason, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&vote.id)
        .bind(&vote.record_id)
        .bind(&vote.candidate_id)
        .bind(&vote.validator)
        .bind(&vote.vote_kind)
        .bind(&vote.reason)
        .bind(&vote.created_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn insert_prompt_version_and_activate(
    pool: &SqlitePool,
    version: &PromptVersionInput<'_>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let normalized_scope_key = normalized_scope_key(version.scope_kind, version.scope_key);

    // Keep old active marker for backwards compatibility and simpler log queries.
    sqlx::query(
        r#"UPDATE evolution_prompt_versions
           SET active = 0, replaced_by = ?
           WHERE scope_kind = ? AND scope_key = ? AND active = 1"#,
    )
    .bind(version.id)
    .bind(version.scope_kind)
    .bind(&normalized_scope_key)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO evolution_prompt_versions
               (id, session_id, scope_kind, scope_key, base_prompt_hash, candidate_prompt, candidate_diff,
                score_before, score_after, active, created_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)"#,
    )
    .bind(version.id)
    .bind(version.session_id)
    .bind(version.scope_kind)
    .bind(&normalized_scope_key)
    .bind(version.base_prompt_hash)
    .bind(version.candidate_prompt)
    .bind(version.candidate_diff)
    .bind(version.score_before)
    .bind(version.score_after)
    .bind(version.created_at)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO evolution_active_scope(scope_kind, scope_key, active_prompt_version_id, updated_at)
           VALUES (?, ?, ?, ?)
           ON CONFLICT(scope_kind, scope_key) DO UPDATE SET
             active_prompt_version_id = excluded.active_prompt_version_id,
             updated_at = excluded.updated_at"#,
    )
    .bind(version.scope_kind)
    .bind(&normalized_scope_key)
    .bind(version.id)
    .bind(version.created_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn set_evolution_enabled(pool: &SqlitePool, enabled: bool) -> Result<()> {
    let value = if enabled { "true" } else { "false" };
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"INSERT INTO evolution_settings(key, value, updated_at)
           VALUES ('enabled', ?, ?)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"#,
    )
    .bind(value)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn is_evolution_enabled(pool: &SqlitePool) -> Result<bool> {
    let row = sqlx::query("SELECT value FROM evolution_settings WHERE key = 'enabled'")
        .fetch_optional(pool)
        .await?;
    let enabled = row
        .and_then(|r| r.try_get::<String, _>("value").ok())
        .unwrap_or_else(|| "true".to_string());
    Ok(enabled == "true")
}

pub async fn resolve_effective_overlay(
    pool: &SqlitePool,
    requested_scope: &EvolutionScope,
) -> Result<Option<EffectiveOverlay>> {
    if matches!(requested_scope.kind, ScopeKind::Workdir) {
        if let Some(workdir) = requested_scope.key.as_deref() {
            if let Some(version) = get_active_prompt_version_for_scope(
                pool,
                ScopeKind::Workdir.as_str(),
                Some(workdir),
            )
            .await?
            {
                return Ok(Some(EffectiveOverlay {
                    selected_scope: ScopeKind::Workdir,
                    selected_scope_key: Some(workdir.to_string()),
                    fallback_used: false,
                    version,
                }));
            }
        }
    }

    let global =
        get_active_prompt_version_for_scope(pool, ScopeKind::Global.as_str(), None).await?;
    Ok(global.map(|version| EffectiveOverlay {
        selected_scope: ScopeKind::Global,
        selected_scope_key: None,
        fallback_used: matches!(requested_scope.kind, ScopeKind::Workdir),
        version,
    }))
}

pub async fn list_prompt_versions(
    pool: &SqlitePool,
    scope_kind: Option<&str>,
    scope_key: Option<&str>,
    limit: i64,
) -> Result<Vec<PromptVersionRecord>> {
    let rows = if scope_kind.is_some() {
        let normalized_scope_key = normalized_scope_key(scope_kind.unwrap_or("global"), scope_key);
        sqlx::query(
            r#"SELECT id, session_id, scope_kind, scope_key, base_prompt_hash, candidate_prompt, candidate_diff,
                      score_before, score_after, active, replaced_by, created_at
               FROM evolution_prompt_versions
               WHERE scope_kind = ? AND scope_key = ?
               ORDER BY created_at DESC
               LIMIT ?"#,
        )
        .bind(scope_kind)
        .bind(normalized_scope_key)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"SELECT id, session_id, scope_kind, scope_key, base_prompt_hash, candidate_prompt, candidate_diff,
                      score_before, score_after, active, replaced_by, created_at
               FROM evolution_prompt_versions
               ORDER BY created_at DESC
               LIMIT ?"#,
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    rows.iter().map(parse_prompt_version_row).collect()
}

pub async fn get_prompt_version_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<PromptVersionRecord>> {
    let row = sqlx::query(
        r#"SELECT id, session_id, scope_kind, scope_key, base_prompt_hash, candidate_prompt, candidate_diff,
                  score_before, score_after, active, replaced_by, created_at
           FROM evolution_prompt_versions
           WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| parse_prompt_version_row(&r)).transpose()
}

pub async fn get_recent_evolution_log(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<EvolutionLogRecord>> {
    let rows = sqlx::query(
        r#"SELECT id, session_id, prompt_score, outcome_kind, outcome_detail, created_at
           FROM evolution_log
           ORDER BY created_at DESC
           LIMIT ?"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter().map(parse_evolution_log_row).collect()
}

pub async fn get_votes_for_record(
    pool: &SqlitePool,
    record_id: &str,
) -> Result<Vec<ValidationVoteRecord>> {
    let rows = sqlx::query(
        r#"SELECT id, record_id, candidate_id, validator, vote_kind, reason, created_at
           FROM evolution_validation_votes
           WHERE record_id = ?
           ORDER BY created_at ASC"#,
    )
    .bind(record_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(parse_vote_row).collect()
}

pub async fn rollback_prompt_version(pool: &SqlitePool, target_id: &str) -> Result<()> {
    let target = get_prompt_version_by_id(pool, target_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("prompt version not found: {target_id}"))?;
    let now = Utc::now().to_rfc3339();
    let normalized_scope_key =
        normalized_scope_key(target.scope_kind.as_str(), target.scope_key.as_deref());

    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"UPDATE evolution_prompt_versions
           SET active = 0, replaced_by = ?
           WHERE scope_kind = ? AND scope_key = ? AND active = 1"#,
    )
    .bind(target.id.as_str())
    .bind(target.scope_kind.as_str())
    .bind(&normalized_scope_key)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE evolution_prompt_versions SET active = 1 WHERE id = ?")
        .bind(target.id.as_str())
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        r#"INSERT INTO evolution_active_scope(scope_kind, scope_key, active_prompt_version_id, updated_at)
           VALUES (?, ?, ?, ?)
           ON CONFLICT(scope_kind, scope_key) DO UPDATE SET
             active_prompt_version_id = excluded.active_prompt_version_id,
             updated_at = excluded.updated_at"#,
    )
    .bind(target.scope_kind.as_str())
    .bind(&normalized_scope_key)
    .bind(target.id.as_str())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

async fn get_active_prompt_version_for_scope(
    pool: &SqlitePool,
    scope_kind: &str,
    scope_key: Option<&str>,
) -> Result<Option<PromptVersionRecord>> {
    let normalized_scope_key = normalized_scope_key(scope_kind, scope_key);
    let active_id = sqlx::query(
        r#"SELECT active_prompt_version_id
           FROM evolution_active_scope
           WHERE scope_kind = ? AND scope_key = ?"#,
    )
    .bind(scope_kind)
    .bind(&normalized_scope_key)
    .fetch_optional(pool)
    .await?
    .and_then(|row| row.try_get::<String, _>("active_prompt_version_id").ok());

    if let Some(id) = active_id {
        return get_prompt_version_by_id(pool, &id).await;
    }

    let fallback = sqlx::query(
        r#"SELECT id, session_id, scope_kind, scope_key, base_prompt_hash, candidate_prompt, candidate_diff,
                  score_before, score_after, active, replaced_by, created_at
           FROM evolution_prompt_versions
           WHERE scope_kind = ? AND scope_key = ? AND active = 1
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(scope_kind)
    .bind(&normalized_scope_key)
    .fetch_optional(pool)
    .await?;
    fallback
        .map(|row| parse_prompt_version_row(&row))
        .transpose()
}

fn parse_prompt_version_row(row: &sqlx::sqlite::SqliteRow) -> Result<PromptVersionRecord> {
    let created_at: String = row.try_get("created_at")?;
    Ok(PromptVersionRecord {
        id: row.try_get("id")?,
        session_id: row.try_get("session_id")?,
        scope_kind: ScopeKind::from_db(&row.try_get::<String, _>("scope_kind")?),
        scope_key: row
            .try_get::<Option<String>, _>("scope_key")?
            .and_then(|v| if v.is_empty() { None } else { Some(v) }),
        base_prompt_hash: row.try_get("base_prompt_hash")?,
        candidate_prompt: row.try_get("candidate_prompt")?,
        candidate_diff: row.try_get("candidate_diff")?,
        score_before: row.try_get("score_before")?,
        score_after: row.try_get("score_after")?,
        active: row.try_get::<i64, _>("active")? != 0,
        replaced_by: row.try_get("replaced_by")?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

fn normalized_scope_key(scope_kind: &str, scope_key: Option<&str>) -> String {
    if scope_kind == "global" {
        String::new()
    } else {
        scope_key.unwrap_or("").to_string()
    }
}

fn parse_evolution_log_row(row: &sqlx::sqlite::SqliteRow) -> Result<EvolutionLogRecord> {
    let created_at: String = row.try_get("created_at")?;
    Ok(EvolutionLogRecord {
        id: row.try_get("id")?,
        session_id: row.try_get("session_id")?,
        prompt_score: row.try_get("prompt_score")?,
        outcome_kind: row.try_get("outcome_kind")?,
        outcome_detail: row.try_get("outcome_detail")?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

fn parse_vote_row(row: &sqlx::sqlite::SqliteRow) -> Result<ValidationVoteRecord> {
    let created_at: String = row.try_get("created_at")?;
    Ok(ValidationVoteRecord {
        id: row.try_get("id")?,
        record_id: row.try_get("record_id")?,
        candidate_id: row.try_get("candidate_id")?,
        validator: row.try_get("validator")?,
        vote_kind: row.try_get("vote_kind")?,
        reason: row.try_get("reason")?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryDb;

    #[tokio::test]
    async fn prompt_version_activation_and_resolution_work() {
        let db = MemoryDb::in_memory().await.expect("in-memory db");
        let first = PromptVersionInput {
            id: "v1",
            session_id: "s1",
            scope_kind: "global",
            scope_key: None,
            base_prompt_hash: "h1",
            candidate_prompt: "prompt one",
            candidate_diff: "diff one",
            score_before: 0.4,
            score_after: None,
            created_at: "2026-04-07T00:00:00Z",
        };
        insert_prompt_version_and_activate(db.pool(), &first)
            .await
            .expect("insert v1");

        let second = PromptVersionInput {
            id: "v2",
            session_id: "s2",
            scope_kind: "global",
            scope_key: None,
            base_prompt_hash: "h2",
            candidate_prompt: "prompt two",
            candidate_diff: "diff two",
            score_before: 0.3,
            score_after: None,
            created_at: "2026-04-07T00:01:00Z",
        };
        insert_prompt_version_and_activate(db.pool(), &second)
            .await
            .expect("insert v2");

        let resolved = resolve_effective_overlay(db.pool(), &EvolutionScope::global())
            .await
            .expect("resolve")
            .expect("overlay");
        assert_eq!(resolved.version.id, "v2");
    }

    #[tokio::test]
    async fn enabled_setting_defaults_true_and_roundtrips() {
        let db = MemoryDb::in_memory().await.expect("in-memory db");
        assert!(is_evolution_enabled(db.pool()).await.expect("read default"));
        set_evolution_enabled(db.pool(), false)
            .await
            .expect("disable");
        assert!(!is_evolution_enabled(db.pool())
            .await
            .expect("read disabled"));
    }
}
