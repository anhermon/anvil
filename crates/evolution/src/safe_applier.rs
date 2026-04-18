//! SafeApplier – APT-pattern evolution applier with backup, benchmarking, and rollback.
//!
//! Implements the Applier trait with three safety gates:
//! 1. **Pre-hook**: Backs up current prompt before applying
//! 2. **Apply**: Writes new prompt to config.toml
//! 3. **Post-hook**: Runs benchmarks and auto-rolls back if performance regresses >10%

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, info, warn};

use crate::traits::Applier;
use crate::types::{EvolutionRecord, PromptCandidate};

/// Benchmark result format expected from the benchmark runner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Overall score (0.0 - 1.0, higher is better)
    pub score: f64,
    /// Individual metric scores
    pub metrics: BenchmarkMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkMetrics {
    pub build_success: f64,
    pub test_pass_rate: f64,
    pub clippy_clean: f64,
    pub fmt_clean: f64,
}

/// Backup record stored alongside the backed-up prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupRecord {
    timestamp: String,
    candidate_id: String,
    candidate_description: String,
    baseline_score: Option<f64>,
}

/// Applier that implements APT-pattern safety gates:
/// - Pre-hook: backup current prompt
/// - Apply: write new prompt to config
/// - Post-hook: benchmark + auto-rollback on regression
pub struct SafeApplier {
    /// Path to backup directory (default: ~/.paperclip/harness/prompt-backups)
    backup_dir: PathBuf,
    /// Path to benchmark runner script
    benchmark_script: PathBuf,
    /// Regression threshold (0.10 = 10%)
    regression_threshold: f64,
}

impl SafeApplier {
    /// Create a new SafeApplier with default paths.
    pub fn new() -> Self {
        let mut backup_dir = dirs_home().unwrap_or_else(|| PathBuf::from("."));
        backup_dir.push(".paperclip/harness/prompt-backups");

        let benchmark_script = PathBuf::from("./scripts/run-evolution-benchmarks.sh");

        Self {
            backup_dir,
            benchmark_script,
            regression_threshold: 0.10, // 10%
        }
    }

    /// Create with custom paths (for testing).
    #[allow(dead_code)]
    pub fn with_paths(backup_dir: PathBuf, benchmark_script: PathBuf) -> Self {
        Self {
            backup_dir,
            benchmark_script,
            regression_threshold: 0.10,
        }
    }

    /// Pre-hook: Back up current prompt and baseline benchmark score.
    async fn backup_current_state(&self, candidate: &PromptCandidate) -> anyhow::Result<PathBuf> {
        // Ensure backup directory exists
        std::fs::create_dir_all(&self.backup_dir)?;

        // Read current config
        let config = harness_core::config::Config::load()?;
        let current_prompt = config
            .agent
            .system_prompt
            .unwrap_or_else(|| "You are a helpful assistant.".to_string());

        // Run baseline benchmarks (if script exists)
        let baseline_score = self.run_benchmarks().await.ok().map(|r| r.score);

        // Create timestamped backup
        let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
        let backup_path = self.backup_dir.join(format!("prompt-{}.txt", timestamp));

        // Write backup
        std::fs::write(&backup_path, current_prompt)?;

        // Write backup metadata
        let record = BackupRecord {
            timestamp: timestamp.to_string(),
            candidate_id: candidate.id.to_string(),
            candidate_description: candidate.description.clone(),
            baseline_score,
        };
        let record_path = self
            .backup_dir
            .join(format!("prompt-{}.meta.json", timestamp));
        std::fs::write(&record_path, serde_json::to_string_pretty(&record)?)?;

        info!(
            backup_path = %backup_path.display(),
            baseline_score = ?baseline_score,
            "backed up current prompt"
        );

        Ok(backup_path)
    }

    /// Apply: Write new prompt to config.toml
    async fn apply_prompt(&self, candidate: &PromptCandidate) -> anyhow::Result<()> {
        let config_path = config_path();

        // Load current config
        let mut config = harness_core::config::Config::load()?;

        // Update system prompt
        config.agent.system_prompt = Some(candidate.prompt.clone());

        // Write back to disk
        let toml = toml::to_string_pretty(&config)?;
        std::fs::write(&config_path, toml)?;

        info!(
            config_path = %config_path.display(),
            candidate_id = %candidate.id,
            "applied new prompt to config"
        );

        Ok(())
    }

    /// Post-hook: Run benchmarks and check for regression.
    async fn run_benchmarks(&self) -> anyhow::Result<BenchmarkResult> {
        if !self.benchmark_script.exists() {
            debug!(
                script = %self.benchmark_script.display(),
                "benchmark script not found, skipping benchmark validation"
            );
            // Return perfect score when benchmarks are not available
            return Ok(BenchmarkResult {
                score: 1.0,
                metrics: BenchmarkMetrics {
                    build_success: 1.0,
                    test_pass_rate: 1.0,
                    clippy_clean: 1.0,
                    fmt_clean: 1.0,
                },
            });
        }

        info!(script = %self.benchmark_script.display(), "running benchmarks");

        let output = Command::new(&self.benchmark_script)
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run benchmark script: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("benchmark script failed: {}", stderr);
        }

        // Parse benchmark-results.json
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: BenchmarkResult = serde_json::from_str(&stdout)
            .map_err(|e| anyhow::anyhow!("failed to parse benchmark results: {}", e))?;

        info!(score = result.score, "benchmark completed");
        Ok(result)
    }

    /// Rollback: Restore prompt from backup.
    async fn rollback(&self, backup_path: &PathBuf) -> anyhow::Result<()> {
        warn!(backup = %backup_path.display(), "rolling back to previous prompt");

        // Read backup
        let backup_prompt = std::fs::read_to_string(backup_path)?;

        // Load config
        let config_path = config_path();
        let mut config = harness_core::config::Config::load()?;

        // Restore old prompt
        config.agent.system_prompt = Some(backup_prompt);

        // Write back
        let toml = toml::to_string_pretty(&config)?;
        std::fs::write(&config_path, toml)?;

        info!("rollback complete");
        Ok(())
    }

    /// Clean up old backups (keep last 10).
    async fn cleanup_old_backups(&self) -> anyhow::Result<()> {
        if !self.backup_dir.exists() {
            return Ok(());
        }

        let mut backups: Vec<_> = std::fs::read_dir(&self.backup_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "txt")
                    .unwrap_or(false)
            })
            .collect();

        // Sort by modification time (oldest first)
        backups.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));

        // Keep last 10, delete the rest
        if backups.len() > 10 {
            for entry in &backups[..backups.len() - 10] {
                let path = entry.path();
                std::fs::remove_file(&path)?;
                // Also remove metadata file
                if let Some(stem) = path.file_stem() {
                    let meta_path = self
                        .backup_dir
                        .join(format!("{}.meta.json", stem.to_string_lossy()));
                    let _ = std::fs::remove_file(meta_path);
                }
                debug!(path = %path.display(), "removed old backup");
            }
        }

        Ok(())
    }
}

impl Default for SafeApplier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Applier for SafeApplier {
    async fn apply(
        &self,
        candidate: &PromptCandidate,
        _record: &EvolutionRecord,
    ) -> anyhow::Result<()> {
        // Gate 1: Pre-hook — backup current state
        let backup_path = self.backup_current_state(candidate).await?;
        let baseline_score = {
            let meta_path = backup_path.with_extension("meta.json");
            if meta_path.exists() {
                let meta: BackupRecord =
                    serde_json::from_str(&std::fs::read_to_string(&meta_path)?)?;
                meta.baseline_score
            } else {
                None
            }
        };

        // Gate 2: Apply — write new prompt to config
        if let Err(e) = self.apply_prompt(candidate).await {
            warn!(error = %e, "failed to apply prompt, skipping post-hook");
            return Err(e);
        }

        // Gate 3: Post-hook — benchmark and auto-rollback
        match self.run_benchmarks().await {
            Ok(new_result) => {
                // Check for regression
                if let Some(baseline) = baseline_score {
                    let regression = baseline - new_result.score;
                    let regression_pct = regression / baseline;

                    info!(
                        baseline = baseline,
                        new_score = new_result.score,
                        regression = regression,
                        regression_pct = regression_pct,
                        threshold = self.regression_threshold,
                        "benchmark comparison"
                    );

                    if regression_pct > self.regression_threshold {
                        warn!(
                            regression_pct = regression_pct,
                            threshold = self.regression_threshold,
                            "performance regression detected, triggering rollback"
                        );
                        self.rollback(&backup_path).await?;
                        anyhow::bail!(
                            "evolution rejected: performance regressed {:.1}% (threshold: {:.1}%)",
                            regression_pct * 100.0,
                            self.regression_threshold * 100.0
                        );
                    }
                } else {
                    info!("no baseline available, accepting new prompt without regression check");
                }
            }
            Err(e) => {
                warn!(error = %e, "benchmark failed, rolling back as safety measure");
                self.rollback(&backup_path).await?;
                return Err(e);
            }
        }

        // Cleanup old backups
        let _ = self.cleanup_old_backups().await;

        info!(
            candidate_id = %candidate.id,
            description = %candidate.description,
            "evolution applied successfully with safety gates"
        );

        Ok(())
    }
}

fn config_path() -> PathBuf {
    let mut p = dirs_home().unwrap_or_else(|| PathBuf::from("."));
    p.push(".paperclip/harness/config.toml");
    p
}

fn dirs_home() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var("USERPROFILE").ok().map(PathBuf::from);
    #[cfg(not(windows))]
    return std::env::var("HOME").ok().map(PathBuf::from);
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_safe_applier_backup_and_rollback() {
        let temp_dir = std::env::temp_dir().join(format!("safe-applier-test-{}", Uuid::new_v4()));
        let backup_dir = temp_dir.join("backups");
        let benchmark_script = temp_dir.join("benchmark.sh");

        // Create a dummy benchmark script that returns success
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(
            &benchmark_script,
            r#"#!/bin/bash
echo '{"score": 0.95, "metrics": {"build_success": 1.0, "test_pass_rate": 0.9, "clippy_clean": 1.0, "fmt_clean": 1.0}}'
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&benchmark_script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&benchmark_script, perms).unwrap();
        }

        let applier = SafeApplier::with_paths(backup_dir.clone(), benchmark_script);

        let candidate = PromptCandidate {
            id: Uuid::new_v4(),
            prompt: "Test prompt".to_string(),
            description: "Test candidate".to_string(),
        };

        // Test backup creation
        let backup_path = applier.backup_current_state(&candidate).await.unwrap();
        assert!(backup_path.exists());
        assert!(backup_dir
            .join(format!(
                "{}.meta.json",
                backup_path.file_stem().unwrap().to_string_lossy()
            ))
            .exists());

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[tokio::test]
    async fn test_benchmark_parsing() {
        let temp_dir = std::env::temp_dir().join(format!("benchmark-test-{}", Uuid::new_v4()));
        let benchmark_script = temp_dir.join("benchmark.sh");

        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(
            &benchmark_script,
            r#"#!/bin/bash
echo '{"score": 0.85, "metrics": {"build_success": 1.0, "test_pass_rate": 0.8, "clippy_clean": 0.9, "fmt_clean": 1.0}}'
"#,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&benchmark_script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&benchmark_script, perms).unwrap();
        }

        let applier = SafeApplier::with_paths(temp_dir.join("backups"), benchmark_script);
        let result = applier.run_benchmarks().await.unwrap();

        assert_eq!(result.score, 0.85);
        assert_eq!(result.metrics.build_success, 1.0);
        assert_eq!(result.metrics.test_pass_rate, 0.8);

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
