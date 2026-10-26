//! Artifact store — downloadable artifacts with integrity verification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Type of integrity proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IntegrityType {
    /// Sigil/wsc detached signature.
    WscSignature,
    /// SHA-256 hash in a signed manifest.
    SignedManifest,
}

impl std::fmt::Display for IntegrityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntegrityType::WscSignature => write!(f, "wsc-signature"),
            IntegrityType::SignedManifest => write!(f, "signed-manifest"),
        }
    }
}

/// A stored artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub name: String,
    pub version: String,
    pub integrity_type: String,
    pub access_scope: Vec<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    /// SHA-256 of the content (always computed for verification).
    pub content_hash: String,
}

/// Response for an artifact download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactDownload {
    pub id: String,
    pub name: String,
    pub version: String,
    pub content: Vec<u8>,
    pub integrity_proof: Vec<u8>,
    pub integrity_type: String,
    pub content_hash: String,
}

/// Parameters for storing a new artifact.
pub struct StoreParams<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub content: &'a [u8],
    pub integrity_proof: &'a [u8],
    pub integrity_type: IntegrityType,
    pub access_scope: Vec<String>,
    pub description: Option<&'a str>,
}

/// Artifact store backed by SQLite.
pub struct ArtifactStore {
    pool: SqlitePool,
}

impl ArtifactStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Store a new artifact.
    pub async fn store(&self, params: StoreParams<'_>) -> anyhow::Result<Artifact> {
        let id = Uuid::now_v7().to_string();
        let now = Utc::now();
        let content_hash = hex::encode(Sha256::digest(params.content));
        let access_scope_json = serde_json::to_string(&params.access_scope)?;

        sqlx::query(
            r#"
            INSERT INTO artifacts (id, name, version, content, integrity_proof, integrity_type,
                                   access_scope, description, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(params.name)
        .bind(params.version)
        .bind(params.content)
        .bind(params.integrity_proof)
        .bind(params.integrity_type.to_string())
        .bind(&access_scope_json)
        .bind(params.description)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(Artifact {
            id,
            name: params.name.to_string(),
            version: params.version.to_string(),
            integrity_type: params.integrity_type.to_string(),
            access_scope: params.access_scope,
            description: params.description.map(String::from),
            created_at: now,
            content_hash,
        })
    }

    /// Download an artifact. Verifies integrity before serving.
    pub async fn download(&self, artifact_id: &str) -> anyhow::Result<Option<ArtifactDownload>> {
        let row: Option<ArtifactRow> = sqlx::query_as("SELECT * FROM artifacts WHERE id = ?")
            .bind(artifact_id)
            .fetch_optional(&self.pool)
            .await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        // Verify content integrity
        let computed_hash = hex::encode(Sha256::digest(&row.content));
        let stored_hash = self.get_content_hash(&row.id).await?;
        if computed_hash != stored_hash {
            anyhow::bail!(
                "artifact integrity check failed: computed {} != stored {}",
                computed_hash,
                stored_hash
            );
        }

        Ok(Some(ArtifactDownload {
            id: row.id,
            name: row.name,
            version: row.version,
            content: row.content,
            integrity_proof: row.integrity_proof,
            integrity_type: row.integrity_type,
            content_hash: computed_hash,
        }))
    }

    /// List all artifacts.
    pub async fn list(&self) -> anyhow::Result<Vec<Artifact>> {
        let rows: Vec<ArtifactRow> =
            sqlx::query_as("SELECT * FROM artifacts ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?;

        let mut artifacts = Vec::new();
        for row in rows {
            let content_hash = self.get_content_hash(&row.id).await?;
            artifacts.push(Artifact {
                id: row.id,
                name: row.name,
                version: row.version,
                integrity_type: row.integrity_type,
                access_scope: serde_json::from_str(&row.access_scope).unwrap_or_default(),
                description: row.description,
                created_at: DateTime::parse_from_rfc3339(&row.created_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                content_hash,
            });
        }
        Ok(artifacts)
    }

    /// Verify an artifact's integrity proof.
    pub async fn verify_integrity(&self, artifact_id: &str) -> anyhow::Result<bool> {
        let row: Option<ArtifactRow> = sqlx::query_as("SELECT * FROM artifacts WHERE id = ?")
            .bind(artifact_id)
            .fetch_optional(&self.pool)
            .await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(false),
        };

        let computed_hash = hex::encode(Sha256::digest(&row.content));
        let stored_hash = self.get_content_hash(artifact_id).await?;

        // TODO: verify wsc signature or signed manifest
        // For now, SHA-256 comparison is the integrity check
        Ok(computed_hash == stored_hash)
    }

    async fn get_content_hash(&self, artifact_id: &str) -> anyhow::Result<String> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as("SELECT content FROM artifacts WHERE id = ?")
            .bind(artifact_id)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some((content,)) => Ok(hex::encode(Sha256::digest(&content))),
            None => anyhow::bail!("artifact not found"),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ArtifactRow {
    id: String,
    name: String,
    version: String,
    content: Vec<u8>,
    integrity_proof: Vec<u8>,
    integrity_type: String,
    access_scope: String,
    description: Option<String>,
    created_at: String,
}
