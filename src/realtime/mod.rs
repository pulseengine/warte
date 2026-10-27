//! Real-time updates — WebSocket and Server-Sent Events.
//!
//! REQ-REALTIME-001: The API SHALL expose a WebSocket endpoint (/api/ws)
//! for real-time updates: agent heartbeats, CI status changes, access
//! revocations, divergence alerts. The dashboard SHALL use Server-Sent
//! Events (/api/events) for unidirectional updates. Both SHALL require
//! authentication (bearer token or mTLS). Updates SHALL include freshness
//! timestamps.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Real-time event types pushed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum RealtimeEvent {
    /// Agent heartbeat received.
    AgentHeartbeat { agent_id: String, health: String },
    /// Agent registered.
    AgentRegistered { agent_id: String, name: String },
    /// Agent health changed.
    AgentHealthChanged {
        agent_id: String,
        old: String,
        new: String,
    },
    /// CI status changed.
    CiStatusChanged {
        repo: String,
        old: String,
        new: String,
    },
    /// Access rights granted.
    AccessGranted {
        agent_id: String,
        rights: Vec<String>,
    },
    /// Access rights revoked.
    AccessRevoked {
        agent_id: String,
        rights: Vec<String>,
    },
    /// Instance divergence detected.
    DivergenceDetected { instance_id: String, distance: u64 },
    /// Artifact uploaded.
    ArtifactUploaded {
        artifact_id: String,
        name: String,
        version: String,
    },
    /// Dashboard overview updated.
    DashboardUpdated { timestamp: DateTime<Utc> },
}

impl RealtimeEvent {
    /// Get the event timestamp (all events are timestamped at send time).
    pub fn timestamp(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Event broadcaster — distributes events to all connected clients.
#[derive(Clone)]
pub struct EventBroadcaster {
    tx: broadcast::Sender<String>,
}

impl EventBroadcaster {
    /// Create a new event broadcaster.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    /// Broadcast an event to all connected clients.
    pub fn broadcast(&self, event: RealtimeEvent) {
        let json = serde_json::json!({
            "event": event,
            "timestamp": Utc::now(),
        });
        if let Ok(json_str) = serde_json::to_string(&json) {
            let _ = self.tx.send(json_str);
        }
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket handler — upgrades HTTP to WebSocket and streams events.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<crate::server::AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, state.realtime))
}

/// Handle a WebSocket connection.
async fn handle_ws(socket: WebSocket, broadcaster: Arc<EventBroadcaster>) {
    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = broadcaster.subscribe();

    // Send initial connection confirmation
    let welcome = serde_json::json!({
        "event": { "type": "connected" },
        "timestamp": Utc::now(),
    });
    if let Ok(welcome_str) = serde_json::to_string(&welcome) {
        let _ = sender.send(Message::Text(welcome_str.into())).await;
    }

    // Spawn a task to forward events to this client
    let mut send_task = tokio::spawn(async move {
        while let Ok(event_str) = event_rx.recv().await {
            if sender.send(Message::Text(event_str.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages (ping/pong, client commands)
    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Parse client command (e.g., subscribe to specific events)
                    tracing::debug!(text = %text, "WS message received");
                }
                Ok(Message::Ping(data)) => {
                    // Axum handles pong automatically
                    let _ = data;
                }
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

/// SSE handler — Server-Sent Events for the dashboard.
pub async fn sse_handler(State(state): State<crate::server::AppState>) -> Response {
    use axum::response::sse::{Event, Sse};
    use axum::response::IntoResponse;
    use std::convert::Infallible;
    use std::time::Duration;

    let event_rx = state.realtime.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(event_rx);
    let stream = tokio_stream::StreamExt::filter_map(stream, |result| {
        result
            .ok()
            .map(|event_str| Ok::<Event, Infallible>(Event::default().data(event_str)))
    });

    Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_broadcaster() {
        let broadcaster = EventBroadcaster::new();
        let mut rx = broadcaster.subscribe();

        broadcaster.broadcast(RealtimeEvent::AgentHeartbeat {
            agent_id: "agent-1".to_string(),
            health: "healthy".to_string(),
        });

        let event_str = rx.try_recv().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&event_str).unwrap();
        assert_eq!(parsed["event"]["type"], "AgentHeartbeat");
        assert_eq!(parsed["event"]["data"]["agent_id"], "agent-1");
        assert!(parsed["timestamp"].is_string());
    }

    #[test]
    fn test_event_serialization() {
        let event = RealtimeEvent::AccessRevoked {
            agent_id: "agent-1".to_string(),
            rights: vec!["artifacts:download".to_string()],
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "AccessRevoked");
        assert_eq!(parsed["data"]["agent_id"], "agent-1");
    }

    #[test]
    fn test_all_event_types() {
        let events = vec![
            RealtimeEvent::AgentHeartbeat {
                agent_id: "a".to_string(),
                health: "healthy".to_string(),
            },
            RealtimeEvent::AgentRegistered {
                agent_id: "a".to_string(),
                name: "test".to_string(),
            },
            RealtimeEvent::AgentHealthChanged {
                agent_id: "a".to_string(),
                old: "healthy".to_string(),
                new: "unhealthy".to_string(),
            },
            RealtimeEvent::CiStatusChanged {
                repo: "rivet".to_string(),
                old: "green".to_string(),
                new: "red".to_string(),
            },
            RealtimeEvent::AccessGranted {
                agent_id: "a".to_string(),
                rights: vec![],
            },
            RealtimeEvent::AccessRevoked {
                agent_id: "a".to_string(),
                rights: vec![],
            },
            RealtimeEvent::DivergenceDetected {
                instance_id: "i1".to_string(),
                distance: 3,
            },
            RealtimeEvent::ArtifactUploaded {
                artifact_id: "art-1".to_string(),
                name: "tool".to_string(),
                version: "1.0".to_string(),
            },
            RealtimeEvent::DashboardUpdated {
                timestamp: Utc::now(),
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert!(!json.is_empty());
        }
    }
}
