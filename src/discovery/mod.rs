//! Service discovery — mDNS/Bonjour for multi-instance warte.
//!
//! REQ-DISCOVERY-001/002/003: Instances discover each other on the local
//! network via mDNS/Bonjour (DNS-SD), without hardcoded peer addresses.
//!
//! Architecture (from docs/research/bonjour-mdns.md):
//! - ADOPT mdns-sd crate for the wire protocol (pure safe Rust, actively maintained)
//! - BUILD discovery semantics: service type _warte-sync._tcp.local
//! - TXT record: id|wg|lan|vv|ts + Ed25519 signature
//! - mDNS scoped to physical LAN only (WireGuard overlay uses static sync_peers)
//! - Fallback ladder: mDNS → static sync_peers → unicast DNS SRV → gossip → CLI
//!
//! REQ-DISCOVERY-003: Full mDNS wire protocol via mdns-sd ServiceDaemon:
//! service registration (announce), browsing (discover), goodbye packets
//! (unregister), TTL-based expiry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Service type for warte sync discovery.
const WARTE_SERVICE_TYPE: &str = "_warte-sync._tcp.local";

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
}

/// Discovery event.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// A new peer was discovered.
    PeerDiscovered(DiscoveredPeer),
    /// A peer was updated (re-announced).
    PeerUpdated(DiscoveredPeer),
    /// A peer went away (goodbye packet or TTL expiry).
    PeerExpired(String),
}

/// mDNS service discovery for warte instances.
///
/// Uses mdns-sd's ServiceDaemon for the full wire protocol:
/// - register(): announce our service
/// - browse(): discover other instances
/// - shutdown(): send goodbye packets and stop
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
    /// mDNS service daemon (created on start).
    daemon: Arc<RwLock<Option<mdns_sd::ServiceDaemon>>>,
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
            daemon: Arc::new(RwLock::new(None)),
        };
        (service, event_rx)
    }

    /// Start the discovery service (announce + browse).
    ///
    /// Uses mdns-sd's ServiceDaemon for the full mDNS wire protocol.
    pub async fn start(&self) -> anyhow::Result<()> {
        let daemon = mdns_sd::ServiceDaemon::new()
            .map_err(|e| anyhow::anyhow!("failed to create mDNS daemon: {}", e))?;

        // Build TXT record properties
        let vv = self.version_vector.read().await;
        let vv_json = serde_json::to_string(&*vv).unwrap_or_else(|_| "{}".to_string());
        drop(vv);

        let mut txt_properties: HashMap<String, String> = HashMap::new();
        txt_properties.insert("id".to_string(), self.instance_id.clone());
        txt_properties.insert("ts".to_string(), chrono::Utc::now().to_rfc3339());
        txt_properties.insert("vv".to_string(), vv_json);
        if let Some(ref wg) = self.wg_address {
            txt_properties.insert("wg".to_string(), wg.clone());
        }
        if let Some(ref lan) = self.lan_address {
            txt_properties.insert("lan".to_string(), lan.clone());
        }

        // Parse LAN address for SRV record
        let (hostname, port) = if let Some(ref lan) = self.lan_address {
            let parts: Vec<&str> = lan.rsplitn(2, ':').collect();
            let port: u16 = parts[0].parse().unwrap_or(8080);
            let host = parts.get(1).unwrap_or(&"").to_string();
            (host, port)
        } else {
            (self.instance_id.clone(), 8080)
        };

        // Register our service (announce)
        let service = mdns_sd::ServiceInfo::new(
            WARTE_SERVICE_TYPE,
            &self.instance_id,
            &hostname,
            "",
            port,
            txt_properties,
        )
        .map_err(|e| anyhow::anyhow!("failed to create service info: {}", e))?;

        daemon
            .register(service)
            .map_err(|e| anyhow::anyhow!("failed to register mDNS service: {}", e))?;

        tracing::info!(
            instance_id = %self.instance_id,
            service_type = WARTE_SERVICE_TYPE,
            "mDNS service registered (announce)"
        );

        // Browse for other instances
        let browse_daemon = daemon.clone();
        let peers = self.peers.clone();
        let event_tx = self.event_tx.clone();
        let shutdown = self.shutdown.clone();
        let my_instance_id = self.instance_id.clone();

        tokio::spawn(async move {
            let receiver = match browse_daemon.browse(WARTE_SERVICE_TYPE) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "failed to browse mDNS services");
                    return;
                }
            };

            while let Ok(event) = receiver.recv() {
                if *shutdown.read().await {
                    break;
                }

                match event {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        let instance_id = info.get_property_val_str("id").unwrap_or("").to_string();
                        if instance_id.is_empty() || instance_id == my_instance_id {
                            continue; // Skip ourselves
                        }

                        let wg_address = info.get_property_val_str("wg").map(String::from);
                        let lan_address =
                            Some(format!("{}:{}", info.get_hostname(), info.get_port()));
                        let vv_str = info.get_property_val_str("vv").unwrap_or("{}");
                        let version_vector: HashMap<String, u64> =
                            serde_json::from_str(vv_str).unwrap_or_default();

                        let peer = DiscoveredPeer {
                            instance_id: instance_id.clone(),
                            wg_address,
                            lan_address,
                            version_vector,
                            last_seen: chrono::Utc::now(),
                        };

                        let is_new = !peers.read().await.contains_key(&instance_id);
                        peers
                            .write()
                            .await
                            .insert(instance_id.clone(), peer.clone());

                        let event = if is_new {
                            DiscoveryEvent::PeerDiscovered(peer)
                        } else {
                            DiscoveryEvent::PeerUpdated(peer)
                        };
                        let _ = event_tx.send(event).await;

                        tracing::info!(instance_id = %instance_id, "mDNS peer discovered");
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                        // Peer went away (goodbye packet)
                        let instance_id = fullname
                            .trim_end_matches(WARTE_SERVICE_TYPE)
                            .trim_end_matches('.')
                            .to_string();
                        peers.write().await.remove(&instance_id);
                        let _ = event_tx
                            .send(DiscoveryEvent::PeerExpired(instance_id.clone()))
                            .await;
                        tracing::info!(instance_id = %instance_id, "mDNS peer expired");
                    }
                    _ => {}
                }
            }
        });

        // Store the daemon for later shutdown
        *self.daemon.write().await = Some(daemon);

        // Spawn expiry checker task
        let expiry_peers = self.peers.clone();
        let expiry_event_tx = self.event_tx.clone();
        let expiry_shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if *expiry_shutdown.read().await {
                            break;
                        }
                        // Check for expired peers (> 2 minutes without announcement)
                        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(2);
                        let mut peers = expiry_peers.write().await;
                        let expired: Vec<String> = peers
                            .iter()
                            .filter(|(_, p)| p.last_seen < cutoff)
                            .map(|(id, _)| id.clone())
                            .collect();
                        for id in expired {
                            peers.remove(&id);
                            let _ = expiry_event_tx.send(DiscoveryEvent::PeerExpired(id)).await;
                        }
                    }
                }
            }
        });

        tracing::info!(
            instance_id = %self.instance_id,
            service_type = WARTE_SERVICE_TYPE,
            "mDNS discovery started (announce + browse)"
        );

        Ok(())
    }

    /// Stop the discovery service (send goodbye packets).
    pub async fn stop(&self) {
        *self.shutdown.write().await = true;
        if let Some(daemon) = self.daemon.write().await.take() {
            let _ = daemon.shutdown();
            tracing::info!(instance_id = %self.instance_id, "mDNS discovery stopped");
        }
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

    #[test]
    fn test_discovery_service_create() {
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
        };

        service.add_static_peer(peer.clone()).await;

        let event = rx.recv().await.unwrap();
        match event {
            DiscoveryEvent::PeerDiscovered(p) => assert_eq!(p.instance_id, "peer-1"),
            _ => panic!("expected PeerDiscovered"),
        }

        let peers = service.get_peers().await;
        assert_eq!(peers.len(), 1);
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
        };

        service.add_static_peer(peer.clone()).await;
        let _ = rx.recv().await;

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
