//! Application State
//!
//! Arc-wrapped state shared across handlers.

use std::sync::Arc;

#[cfg(debug_assertions)]
use tokio::sync::broadcast;

use crate::content::ContentStore;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<InnerState>,
}

struct InnerState {
    pub content: ContentStore,
    #[cfg(debug_assertions)]
    pub reloader: Option<broadcast::Sender<()>>,
}

impl AppState {
    /// Create a new `AppState` without hot reload.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(InnerState {
                content: ContentStore::load(),
                #[cfg(debug_assertions)]
                reloader: None,
            }),
        }
    }

    /// Create a new `AppState` with hot reload channel (debug only).
    #[cfg(debug_assertions)]
    pub fn with_reloader(self) -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            inner: Arc::new(InnerState {
                content: self.inner.content.clone(),
                reloader: Some(tx),
            }),
        }
    }

    /// Get the content store.
    pub fn content(&self) -> &ContentStore {
        &self.inner.content
    }

    /// Get the reloader channel (debug only).
    #[cfg(debug_assertions)]
    pub fn reloader(&self) -> Option<&broadcast::Sender<()>> {
        self.inner.reloader.as_ref()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
