//! Relay contract patterns — thin interfaces and seam traits.
//!
//! REQ-REUSE-001: Reuse relay's architectural patterns for agent orchestration:
//! - Thin agreed interfaces (contracts) between agents and the control tower
//! - HAL-style seam traits for platform abstraction (CiPlatform already implements this)
//!
//! From the relay/jess contracts (jess #62, jess #167, relay #572):
//! - HAL contract: relay owns the abstraction (seam traits), jess owns the binding
//! - Consumption contract: thin agreed interface for consuming WASM components
//! - Interface identity: distinct interface per component
//!
//! Applied to warte:
//! - AgentContract: the thin interface between an agent and the control tower
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
    /// The token right the route requires (REQ-API-004), if it takes a token.
    /// Routes naming an agent (`{id}`, register) also bind the token's agent
    /// id to that agent unless the token holds `agents:any`.
    pub required_right: Option<String>,
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

/// The right the auth table requires for a route pattern (REQ-API-004), so
/// the contract cannot list one the middleware does not enforce.
fn right_for(pattern: &str) -> Option<String> {
    let rule = crate::auth::MUTATING_ROUTES
        .iter()
        .find(|r| r.pattern == pattern)
        .unwrap_or_else(|| panic!("{pattern} is not in auth::MUTATING_ROUTES"));
    Some(rule.right.to_string())
}

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
                // Since #116 a registration needs a minted token too.
                auth_required: true,
                required_right: right_for("/api/agents/register"),
            },
            ContractOperation {
                name: "heartbeat".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "()".to_string(),
                response_type: "()".to_string(),
                auth_required: true,
                required_right: right_for("/api/agents/{id}/heartbeat"),
            },
            ContractOperation {
                name: "github_token".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "GithubTokenRequest".to_string(),
                response_type: "GithubToken".to_string(),
                auth_required: true,
                required_right: right_for("/api/agents/{id}/github-token"),
            },
            ContractOperation {
                name: "capture_data".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "CaptureDataRequest".to_string(),
                response_type: "CaptureDataResponse".to_string(),
                auth_required: true,
                required_right: right_for("/api/agents/{id}/data"),
            },
            ContractOperation {
                name: "query_data".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "QueryDataRequest".to_string(),
                response_type: "QueryDataResponse".to_string(),
                auth_required: true,
                required_right: None, // a read route: public (#79)
            },
            ContractOperation {
                name: "grant_access".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "GrantRequest".to_string(),
                response_type: "{ policy_ids: string[] }".to_string(),
                auth_required: true,
                required_right: right_for("/api/access/grant"),
            },
            ContractOperation {
                name: "revoke_access".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "RevokeRequest".to_string(),
                response_type: "()".to_string(),
                auth_required: true,
                required_right: right_for("/api/access/revoke"),
            },
            ContractOperation {
                name: "post_costs".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "CostReport".to_string(),
                response_type: "{ stored: bool, sessions: number }".to_string(),
                auth_required: true,
                required_right: right_for("/api/costs"),
            },
            ContractOperation {
                name: "post_skill_usage".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "SkillUsageReport".to_string(),
                response_type: "{ stored: bool, source: string, skills: number }".to_string(),
                auth_required: true,
                required_right: right_for("/api/skill-usage"),
            },
            ContractOperation {
                name: "post_briefing".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "{ from: string, model: string, line: string, bullets: [string; 3] }"
                    .to_string(),
                response_type: "{ stored: bool }".to_string(),
                auth_required: true,
                required_right: right_for("/api/briefing"),
            },
            ContractOperation {
                name: "post_triage_done".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "{ repo: string, number: number, outcome: replied|asked|skipped }"
                    .to_string(),
                response_type: "{ done: bool }".to_string(),
                auth_required: true,
                required_right: right_for("/api/triage/done"),
            },
            ContractOperation {
                name: "post_news_state".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type:
                    "{ id: number, state: found|decided|landed|dismissed, note: string? }"
                        .to_string(),
                response_type: "{ id: number, state: string }".to_string(),
                auth_required: true,
                required_right: right_for("/api/news/state"),
            },
            ContractOperation {
                name: "post_report_seam_check".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "seam-check.v1 (the report seam-check.py --format json writes)".to_string(),
                response_type: "204, or 422 naming the field".to_string(),
                auth_required: true,
                required_right: right_for("/api/reports/seam-check"),
            },
            ContractOperation {
                name: "post_alert_triage_verdict".to_string(),
                direction: OperationDirection::AgentToTower,
                request_type: "{ id, verdict: real|noise, reason, model } (warte.alert-triage.v1)".to_string(),
                response_type: "204; 404 unknown alert; 409 already decided".to_string(),
                auth_required: true,
                required_right: right_for("/api/alerts/triage/verdict"),
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
                name: "CapturedData".to_string(),
                schema: "{ agent_id: string, data_type: string, content: string, captured_at: datetime }".to_string(),
                integrity_guarantee: IntegrityGuarantee::None,
            },
        ],
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
        assert_eq!(contract.operations.len(), 14);
        assert_eq!(contract.data_types.len(), 3);

        // Every operation requires auth; every mutating route's right is listed.
        for op in &contract.operations {
            assert!(op.auth_required, "{} should require auth", op.name);
        }
        for rule in crate::auth::MUTATING_ROUTES {
            assert!(
                contract
                    .operations
                    .iter()
                    .any(|op| op.required_right.as_deref() == Some(rule.right)),
                "contract omits {}",
                rule.right
            );
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
