//! Relay contract patterns — thin interfaces, seam traits, consumption contracts.
//!
//! REQ-REUSE-001: Reuse relay's architectural patterns for agent orchestration:
//! - Thin agreed interfaces (contracts) between agents and the control tower
//! - HAL-style seam traits for platform abstraction (CiPlatform already implements this)
//! - Consumption contracts for artifact distribution
//!
//! From the relay/jess contracts (jess #62, jess #167, relay #572):
//! - HAL contract: relay owns the abstraction (seam traits), jess owns the binding
//! - Consumption contract: thin agreed interface for consuming WASM components
//! - Interface identity: distinct interface per component
//!
//! Applied to warte:
//! - AgentContract: the thin interface between an agent and the control tower
//! - ConsumptionContract: what an agent can download and how it verifies it
//! - CiPlatform: seam trait for CI platform abstraction (already implemented)

use serde::{Deserialize, Serialize};

/// The thin contract between an agent and the control tower.
///
/// This is the warte equivalent of relay's HAL contract (jess #62):
/// the abstraction is owned by the control tower, the binding is owned
/// by the agent. The contract is thin, agreed, and bidirectional.
///
/// An agent MUST NOT break this contract without the control tower knowing.
/// The control tower MUST NOT break this contract without agents knowing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContract {
    /// Contract version (semver — breaking changes bump major).
    pub version: String,
    /// Operations the agent can perform.
    pub operations: Vec<ContractOperation>,
    /// Data types exchanged.
    pub data_types: Vec<ContractDataType>,
}

/// A single operation in the agent contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractOperation {
    pub name: String,
    pub direction: OperationDirection,
    pub request_type: String,
    pub response_type: String,
    pub auth_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OperationDirection {
    AgentToTower,
    TowerToAgent,
    Bidirectional,
}

/// A data type in the agent contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractDataType {
    pub name: String,
    pub schema: String,
    pub integrity_guarantee: IntegrityGuarantee,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IntegrityGuarantee {
    /// No integrity guarantee (plain data).
    None,
    /// SHA-256 hash verification.
    Sha256,
    /// Sigil/wsc detached signature.
    WscSignature,
}

/// The current agent contract version.
pub const AGENT_CONTRACT_VERSION: &str = "0.1.0";

/// Get the current agent contract.
pub fn current_contract() -> AgentContract {
    AgentContract {
        version: AGENT_CONTRACT_VERSION.to_string(),
        operations: vec![
            ContractOperation {
                name: "register".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "RegisterRequest".to_string(),
                response_type: "RegisterResponse".to_string(),
                auth_required: false, // Registration is the auth bootstrap
            },
            ContractOperation {
                name: "heartbeat".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "()".to_string(),
                response_type: "()".to_string(),
                auth_required: true,
            },
            ContractOperation {
                name: "download_artifact".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "ArtifactDownloadRequest".to_string(),
                response_type: "ArtifactDownload".to_string(),
                auth_required: true,
            },
            ContractOperation {
                name: "capture_data".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "CaptureDataRequest".to_string(),
                response_type: "CaptureDataResponse".to_string(),
                auth_required: true,
            },
            ContractOperation {
                name: "query_data".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "QueryDataRequest".to_string(),
                response_type: "QueryDataResponse".to_string(),
                auth_required: true,
            },
        ],
        data_types: vec![
            ContractDataType {
                name: "RegisterRequest".to_string(),
                schema: "{ name: string, repo: string?, identity_type: string, public_key: string }".to_string(),
                integrity_guarantee: IntegrityGuarantee::None,
            },
            ContractDataType {
                name: "RegisterResponse".to_string(),
                schema: "{ agent_id: string, cert_pem: string?, cert_expires_at: datetime?, access_rights: string[] }".to_string(),
                integrity_guarantee: IntegrityGuarantee::WscSignature,
            },
            ContractDataType {
                name: "ArtifactDownload".to_string(),
                schema: "{ id: string, name: string, version: string, content: bytes, integrity_proof: bytes, content_hash: string }".to_string(),
                integrity_guarantee: IntegrityGuarantee::Sha256,
            },
            ContractDataType {
                name: "CapturedData".to_string(),
                schema: "{ agent_id: string, data_type: string, content: string, captured_at: datetime }".to_string(),
                integrity_guarantee: IntegrityGuarantee::None,
            },
        ],
    }
}

/// Consumption contract — what an agent can download and how it verifies it.
///
/// This is the warte equivalent of jess #167 (consumption contract):
/// a thin agreed interface for consuming artifacts. The agent knows
/// exactly what it can download, what integrity guarantee it gets,
/// and how to verify it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumptionContract {
    /// Contract version.
    pub version: String,
    /// Artifact types the agent can consume.
    pub consumable_types: Vec<ConsumableType>,
    /// Verification procedure.
    pub verification: VerificationProcedure,
}

/// A type of artifact an agent can consume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumableType {
    pub name: String,
    pub integrity_guarantee: IntegrityGuarantee,
    pub access_scope: Vec<String>,
}

/// How the agent verifies an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationProcedure {
    pub steps: Vec<String>,
}

/// Get the current consumption contract.
pub fn current_consumption_contract() -> ConsumptionContract {
    ConsumptionContract {
        version: "0.1.0".to_string(),
        consumable_types: vec![
            ConsumableType {
                name: "tool".to_string(),
                integrity_guarantee: IntegrityGuarantee::Sha256,
                access_scope: vec!["artifacts:download".to_string()],
            },
            ConsumableType {
                name: "config".to_string(),
                integrity_guarantee: IntegrityGuarantee::Sha256,
                access_scope: vec!["artifacts:download".to_string()],
            },
            ConsumableType {
                name: "data".to_string(),
                integrity_guarantee: IntegrityGuarantee::Sha256,
                access_scope: vec!["artifacts:download".to_string()],
            },
        ],
        verification: VerificationProcedure {
            steps: vec![
                "Download artifact from /api/artifacts/{id}/download".to_string(),
                "Verify SHA-256 hash: sha256(content) == content_hash".to_string(),
                "If integrity_type is wsc-signature, verify with wsc verify".to_string(),
                "Reject artifact if verification fails".to_string(),
            ],
        },
    }
}

/// Seam trait for agent-side platform abstraction.
///
/// This is the warte equivalent of relay's HAL seam traits (jess #62):
/// the control tower owns the abstraction (the trait), the agent owns
/// the binding (the implementation). The seam is thin and agreed.
///
/// An agent implements this trait to communicate with the control tower.
/// The trait is the contract; the implementation is the binding.
#[async_trait::async_trait]
pub trait AgentSeam: Send + Sync {
    /// Register with the control tower.
    async fn register(&self, name: &str, repo: Option<&str>) -> anyhow::Result<String>;

    /// Send a heartbeat.
    async fn heartbeat(&self) -> anyhow::Result<()>;

    /// Download an artifact.
    async fn download(&self, artifact_id: &str) -> anyhow::Result<Vec<u8>>;

    /// Capture data.
    async fn capture(&self, data_type: &str, content: &str) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_contract() {
        let contract = current_contract();
        assert_eq!(contract.version, "0.1.0");
        assert_eq!(contract.operations.len(), 5);
        assert_eq!(contract.data_types.len(), 4);

        // Registration must not require auth (it's the bootstrap)
        let register = contract
            .operations
            .iter()
            .find(|op| op.name == "register")
            .unwrap();
        assert!(!register.auth_required);

        // All other operations require auth
        for op in &contract.operations {
            if op.name != "register" {
                assert!(op.auth_required, "{} should require auth", op.name);
            }
        }
    }

    #[test]
    fn test_consumption_contract() {
        let contract = current_consumption_contract();
        assert_eq!(contract.version, "0.1.0");
        assert_eq!(contract.consumable_types.len(), 3);
        assert_eq!(contract.verification.steps.len(), 4);

        // All consumable types require SHA-256 integrity
        for ct in &contract.consumable_types {
            assert_eq!(ct.integrity_guarantee, IntegrityGuarantee::Sha256);
        }
    }

    #[test]
    fn test_contract_serialization() {
        let contract = current_contract();
        let json = serde_json::to_string(&contract).unwrap();
        let parsed: AgentContract = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, contract.version);
        assert_eq!(parsed.operations.len(), contract.operations.len());
    }
}
