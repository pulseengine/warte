//! Service discovery — mDNS/Bonjour for multi-instance warte.
//!
//! REQ-DISCOVERY-001: Instances discover each other on the local network
//! via mDNS/Bonjour (DNS-SD), without hardcoded peer addresses.
//!
//! REQ-DISCOVERY-002: mDNS is for discovery only, not authentication.
//! Discovered peers still authenticate via mTLS before sync.
//!
//! Architecture (from docs/research/bonjour-mdns.md):
//! - ADOPT mdns-sd crate for the wire protocol (pure safe Rust, actively maintained)
//! - BUILD discovery semantics: service type _warte-sync._tcp.local
//! - TXT record: id|wg|lan|vv|ts + Ed25519 signature
//! - mDNS scoped to physical LAN only (WireGuard overlay uses static sync_peers)
//! - Fallback ladder: mDNS → static sync_peers → unicast DNS SRV → gossip → CLI

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

/// Service type for warte sync discovery.
const WARTE_SERVICE_TYPE: &str = "_warte-sync._tcp.local";

/// TXT record keys.
const TXT_KEY_ID: &str = "id";
const TXT_KEY_WG: &str = "wg";
const TXT_KEY_LAN: &str = "lan";
const TXT_KEY_VV: &str = "vv";
const TXT_KEY_TS: &str = "ts";
#[allow(dead_code)]
const TXT_KEY_SIG: &str = "sig";

/// A discovered warte peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    /// Instance ID (from TXT record).
    pub instance_id: String,
    /// WireGuard address (from TXT record, e.g. "10.0.0.2:8080").
    pub wg_address: Option<String>,
    /// LAN address (from mDNS SRV record).
    pub lan_address: Option<String>,
    /// Version vector (from TXT record, JSON-encoded).
    pub version_vector: HashMap<String, u64>,
    /// Timestamp of last announcement.
    pub last_seen: chrono::DateTime<chrono::Utc>,
    /// Ed25519 signature over the TXT record (for attestation).
    pub signature: Option<String>,
}

/// Discovery event.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// A new peer was discovered.
    PeerDiscovered(DiscoveredPeer),
    /// A peer was updated (re-announced).
    PeerUpdated(DiscoveredPeer),
    /// A peer went away (goodbye packet).
    PeerExpired(String),
}

/// mDNS service discovery for warte instances.
pub struct DiscoveryService {
    /// Our instance ID.
    instance_id: String,
    /// Our WireGuard sync address (advertised in TXT).
    wg_address: Option<String>,
    /// Our LAN sync address (advertised via SRV).
    lan_address: Option<String>,
    /// Current version vector (advertised in TXT).
    version_vector: Arc<RwLock<HashMap<String, u64>>>,
    /// Discovered peers.
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
    /// Event sender.
    event_tx: mpsc::Sender<DiscoveryEvent>,
    /// Shutdown signal.
    shutdown: Arc<RwLock<bool>>,
}

impl DiscoveryService {
    /// Create a new discovery service.
    pub fn new(
        instance_id: String,
        wg_address: Option<String>,
        lan_address: Option<String>,
    ) -> (Self, mpsc::Receiver<DiscoveryEvent>) {
        let (event_tx, event_rx) = mpsc::channel(64);
        let service = Self {
            instance_id,
            wg_address,
            lan_address,
            version_vector: Arc::new(RwLock::new(HashMap::new())),
            peers: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            shutdown: Arc::new(RwLock::new(false)),
        };
        (service, event_rx)
    }

    /// Start the discovery service (announce + browse).
    ///
    /// This spawns two background tasks:
    /// 1. Announce: register our service with mDNS
    /// 2. Browse: listen for other warte instances
    pub async fn start(&self) -> anyhow::Result<()> {
        let instance_id = self.instance_id.clone();
        let wg_address = self.wg_address.clone();
        let lan_address = self.lan_address.clone();
        let version_vector = self.version_vector.clone();
        let peers = self.peers.clone();
        let event_tx = self.event_tx.clone();
        let shutdown = self.shutdown.clone();

        // Build TXT record
        let vv = version_vector.read().await;
        let vv_json = serde_json::to_string(&*vv).unwrap_or_else(|_| "{}".to_string());
        drop(vv);

        let mut txt_properties = vec![
            (TXT_KEY_ID.to_string(), instance_id.clone()),
            (TXT_KEY_TS.to_string(), chrono::Utc::now().to_rfc3339()),
            (TXT_KEY_VV.to_string(), vv_json),
        ];
        if let Some(ref wg) = wg_address {
            txt_properties.push((TXT_KEY_WG.to_string(), wg.clone()));
        }
        if let Some(ref lan) = lan_address {
            txt_properties.push((TXT_KEY_LAN.to_string(), lan.clone()));
        }

        // Spawn browse task
        let browse_peers = peers.clone();
        let browse_event_tx = event_tx.clone();
        let browse_shutdown = shutdown.clone();
        let browse_instance_id = instance_id.clone();
        tokio::spawn(async move {
            Self::browse_task(
                browse_instance_id,
                browse_peers,
                browse_event_tx,
                browse_shutdown,
            )
            .await;
        });

        // Spawn announce task (simplified — in production, use mdns-sd's ServiceDaemon)
        let announce_shutdown = shutdown.clone();
        tokio::spawn(async move {
            Self::announce_task(txt_properties, announce_shutdown).await;
        });

        tracing::info!(
            instance_id = %instance_id,
            service_type = WARTE_SERVICE_TYPE,
            "mDNS discovery started"
        );

        Ok(())
    }

    /// Browse task — listen for other warte instances on the LAN.
    async fn browse_task(
        _instance_id: String,
        peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
        event_tx: mpsc::Sender<DiscoveryEvent>,
        shutdown: Arc<RwLock<bool>>,
    ) {
        // In production, this uses mdns-sd::ServiceDaemon::browse()
        // For now, this is a placeholder that demonstrates the structure.
        // The actual mDNS browsing is implemented when mdns-sd is integrated.

        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if *shutdown.read().await {
                        break;
                    }
                    // Check for expired peers (> 2 minutes without announcement)
                    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(2);
                    let mut peers = peers.write().await;
                    let expired: Vec<String> = peers
                        .iter()
                        .filter(|(_, p)| p.last_seen < cutoff)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in expired {
                        peers.remove(&id);
                        let _ = event_tx.send(DiscoveryEvent::PeerExpired(id)).await;
                    }
                }
            }
        }
    }

    /// Announce task — register our service with mDNS.
    async fn announce_task(_txt_properties: Vec<(String, String)>, shutdown: Arc<RwLock<bool>>) {
        // In production, this uses mdns-sd::ServiceDaemon::register()
        // For now, this is a placeholder.
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if *shutdown.read().await {
                        break;
                    }
                    // Re-announce (mDNS TTL expiry)
                }
            }
        }
    }

    /// Stop the discovery service.
    pub async fn stop(&self) {
        *self.shutdown.write().await = true;
    }

    /// Get all discovered peers.
    pub async fn get_peers(&self) -> Vec<DiscoveredPeer> {
        self.peers.read().await.values().cloned().collect()
    }

    /// Update our version vector (advertised in TXT record).
    pub async fn update_version_vector(&self, vv: HashMap<String, u64>) {
        *self.version_vector.write().await = vv;
    }

    /// Manually add a peer (from static config or DNS SRV fallback).
    pub async fn add_static_peer(&self, peer: DiscoveredPeer) {
        let is_new = !self.peers.read().await.contains_key(&peer.instance_id);
        self.peers
            .write()
            .await
            .insert(peer.instance_id.clone(), peer.clone());
        let event = if is_new {
            DiscoveryEvent::PeerDiscovered(peer)
        } else {
            DiscoveryEvent::PeerUpdated(peer)
        };
        let _ = self.event_tx.send(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_service_create() {
        let (service, _rx) = DiscoveryService::new(
            "test-instance".to_string(),
            Some("10.0.0.2:8080".to_string()),
            Some("192.168.1.10:8080".to_string()),
        );
        assert_eq!(service.instance_id, "test-instance");
        assert_eq!(service.wg_address, Some("10.0.0.2:8080".to_string()));
    }

    #[tokio::test]
    async fn test_discovery_add_static_peer() {
        let (service, mut rx) = DiscoveryService::new("test-instance".to_string(), None, None);

        let peer = DiscoveredPeer {
            instance_id: "peer-1".to_string(),
            wg_address: Some("10.0.0.3:8080".to_string()),
            lan_address: Some("192.168.1.20:8080".to_string()),
            version_vector: HashMap::new(),
            last_seen: chrono::Utc::now(),
            signature: None,
        };

        service.add_static_peer(peer.clone()).await;

        // Should receive PeerDiscovered event
        let event = rx.recv().await.unwrap();
        match event {
            DiscoveryEvent::PeerDiscovered(p) => assert_eq!(p.instance_id, "peer-1"),
            _ => panic!("expected PeerDiscovered"),
        }

        // Peer should be in the list
        let peers = service.get_peers().await;
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].instance_id, "peer-1");
    }

    #[tokio::test]
    async fn test_discovery_update_peer() {
        let (service, mut rx) = DiscoveryService::new("test-instance".to_string(), None, None);

        let peer = DiscoveredPeer {
            instance_id: "peer-1".to_string(),
            wg_address: None,
            lan_address: None,
            version_vector: HashMap::new(),
            last_seen: chrono::Utc::now(),
            signature: None,
        };

        // Add peer (discovered)
        service.add_static_peer(peer.clone()).await;
        let _ = rx.recv().await;

        // Update peer (updated)
        let mut updated = peer.clone();
        updated.last_seen = chrono::Utc::now();
        service.add_static_peer(updated).await;

        let event = rx.recv().await.unwrap();
        match event {
            DiscoveryEvent::PeerUpdated(p) => assert_eq!(p.instance_id, "peer-1"),
            _ => panic!("expected PeerUpdated"),
        }
    }
}
