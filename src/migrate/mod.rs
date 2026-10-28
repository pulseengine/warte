//! Migration commands — import repo hardening config from temper.
//!
//! REQ-MIGRATE-002: Migrate repo hardening config from temper (GitHub App).
//!
//! smithy hub needs no import: `warte hub` reads its data directory as-is
//! (DD-SMITHY-001). The earlier `migrate_smithy` parsed a format smithy never
//! had and was removed.
//!
//! Temper is linked, not absorbed — we read its config.yml and convert
//! to warte's access control policy format.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::Path;

/// Result of a migration operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationResult {
    pub source: String,
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub checksum: String,
}

/// Migrate repo hardening config from temper.
///
/// Temper config format (config.yml):
/// ```yaml
/// organization: pulseengine
/// branch_protection:
///   default:
///     required_status_checks:
///       contexts: ['ci', 'lint', 'test']
///     require_signed_commits: true
/// ```
///
/// We convert this to warte access policies: each repo gets an access
/// policy entry mapping temper's settings to warte's RBAC model.
pub async fn migrate_temper(
    pool: &SqlitePool,
    config_path: &Path,
) -> anyhow::Result<MigrationResult> {
    let content = std::fs::read_to_string(config_path)?;
    let temper_config: TemperConfig = serde_yaml::from_str(&content)?;

    let mut imported = 0;
    let skipped = 0;
    let mut errors = Vec::new();

    // Convert temper's branch protection to warte access policies
    if let Some(bp) = &temper_config.branch_protection {
        if let Some(default) = &bp.default {
            // Create a synthetic "temper-migrated" agent representing the
            // org-level policy, then grant it admin rights
            let policy_agent_id = "temper-migrated-policy";

            // Ensure the policy agent exists
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO agents (id, name, repo, identity_type, public_key,
                                              health_status, instance_id, created_at, updated_at)
                VALUES (?, 'temper-policy', NULL, 'migrated', 'temper-config', 'unknown',
                        'migration', datetime('now'), datetime('now'))
                "#,
            )
            .bind(policy_agent_id)
            .execute(pool)
            .await?;

            // Grant admin rights (temper manages the whole org)
            let rights = vec![crate::access::AccessRight::new("*", "*")];
            let access = crate::access::AccessControl::new(pool.clone(), "migration".to_string());
            match access
                .grant(policy_agent_id, &rights, "temper-migration")
                .await
            {
                Ok(_) => imported += 1,
                Err(e) => errors.push(format!("grant admin: {}", e)),
            }

            // Record the branch protection settings as captured data
            let bp_json = serde_json::to_string_pretty(default)?;
            sqlx::query(
                r#"
                INSERT INTO agent_data (id, agent_id, data_type, content, captured_at, instance_id)
                VALUES (?, ?, 'temper-branch-protection', ?, datetime('now'), 'migration')
                "#,
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(policy_agent_id)
            .bind(&bp_json)
            .execute(pool)
            .await?;
            imported += 1;
        }
    }

    // Convert issue labels to captured data
    if let Some(labels) = &temper_config.issue_labels {
        let labels_json = serde_json::to_string_pretty(labels)?;
        sqlx::query(
            r#"
            INSERT INTO agent_data (id, agent_id, data_type, content, captured_at, instance_id)
            VALUES (?, 'temper-migrated-policy', 'temper-issue-labels', ?, datetime('now'), 'migration')
            "#,
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(&labels_json)
        .execute(pool)
        .await?;
        imported += 1;
    }

    let checksum = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(content.as_bytes()))
    };

    Ok(MigrationResult {
        source: "temper".to_string(),
        imported,
        skipped,
        errors,
        checksum,
    })
}

/// Temper config format (subset).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TemperConfig {
    #[serde(default)]
    organization: Option<String>,
    #[serde(default)]
    branch_protection: Option<TemperBranchProtection>,
    #[serde(default)]
    issue_labels: Option<Vec<TemperLabel>>,
}

#[derive(Debug, Deserialize)]
struct TemperBranchProtection {
    #[serde(default)]
    default: Option<TemperBPDefault>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TemperBPDefault {
    #[serde(default)]
    required_status_checks: Option<serde_json::Value>,
    #[serde(default)]
    require_signed_commits: Option<bool>,
    #[serde(default)]
    required_pull_request_reviews: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TemperLabel {
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_migrate_temper() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = crate::db::Database::connect(&db_path).await.unwrap();

        // Create a temper config file
        let temper_config = r#"
organization: pulseengine
branch_protection:
  default:
    required_status_checks:
      contexts: ['ci', 'lint', 'test']
    require_signed_commits: true
issue_labels:
  - name: bug
    color: red
  - name: feature
    color: green
"#;
        let temper_path = dir.path().join("config.yml");
        std::fs::write(&temper_path, temper_config).unwrap();

        let result = migrate_temper(db.pool(), &temper_path).await.unwrap();
        assert_eq!(result.source, "temper");
        assert!(result.imported >= 2); // policy agent + branch protection + labels
        assert!(result.errors.is_empty());
    }
}
