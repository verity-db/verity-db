//! Development Tools
//!
//! Hot reload via SSE for development.

use std::path::Path;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::state::AppState;

/// SSE endpoint for live reload.
pub async fn livereload_handler(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(16);

    if let Some(reloader) = state.reloader() {
        let mut receiver = reloader.subscribe();

        tokio::spawn(async move {
            while receiver.recv().await.is_ok() {
                if tx.send(Ok(Event::default().data("reload"))).await.is_err() {
                    break;
                }
            }
        });
    }

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

/// Spawn file watcher for hot reload.
pub fn spawn_file_watcher(state: AppState) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher = RecommendedWatcher::new(tx, Config::default()).expect("failed to create watcher");

        let paths_to_watch = ["templates", "public"];

        for path in paths_to_watch {
            let watch_path = Path::new(path);
            if watch_path.exists() {
                if let Err(e) = watcher.watch(watch_path, RecursiveMode::Recursive) {
                    error!("Failed to watch {}: {}", path, e);
                } else {
                    info!("Watching {} for changes", path);
                }
            }
        }

        loop {
            match rx.recv() {
                Ok(Ok(event)) => {
                    if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                        debug!("File change detected: {:?}", event.paths);
                        if let Some(reloader) = state.reloader() {
                            let _ = reloader.send(());
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!("Watch error: {:?}", e);
                }
                Err(e) => {
                    error!("Channel error: {:?}", e);
                    break;
                }
            }
        }
    });
}
