//! Action router — routes user responses back to the correct agent session.

use crate::event::{AttentionEvent, UserResponse};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

/// Error type for routing operations.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("No pending request found for event {0}")]
    EventNotFound(Uuid),
    #[error("Response channel closed for event {0}")]
    ChannelClosed(Uuid),
}

/// Manages pending attention events and routes user responses.
#[derive(Debug, Clone)]
pub struct ActionRouter {
    /// Map of event_id → oneshot sender for the response
    pending: Arc<RwLock<HashMap<Uuid, oneshot::Sender<UserResponse>>>>,
}

impl ActionRouter {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register(&self, event: &AttentionEvent) -> oneshot::Receiver<UserResponse> {
        let (tx, rx) = oneshot::channel();
        self.pending.write().await.insert(event.event_id, tx);
        info!(event_id = %event.event_id, "Registered pending attention event");
        rx
    }

    pub async fn resolve(&self, response: UserResponse) -> Result<(), RouterError> {
        let event_id = response.event_id;
        let sender = self
            .pending
            .write()
            .await
            .remove(&event_id)
            .ok_or(RouterError::EventNotFound(event_id))?;

        sender
            .send(response)
            .map_err(|_| RouterError::ChannelClosed(event_id))?;

        info!(event_id = %event_id, "Resolved attention event");
        Ok(())
    }

    pub async fn cancel(&self, event_id: &Uuid) -> bool {
        let removed = self.pending.write().await.remove(event_id).is_some();
        if removed {
            warn!(event_id = %event_id, "Cancelled pending attention event");
        }
        removed
    }

    pub async fn pending_count(&self) -> usize {
        self.pending.read().await.len()
    }
}

impl Default for ActionRouter {
    fn default() -> Self {
        Self::new()
    }
}
