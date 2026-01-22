//! Router Configuration
//!
//! Route configuration for the website.

use axum::{routing::get, Router};
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{handlers, state::AppState};

/// Create the main router with all routes.
pub fn create_router(state: AppState) -> Router {
    let router = Router::new()
        .route("/", get(handlers::home::home))
        .route("/blog", get(handlers::blog::blog_index))
        .route("/blog/{slug}", get(handlers::blog::blog_post))
        .nest_service("/public", ServeDir::new("public"))
        .layer(TraceLayer::new_for_http());

    #[cfg(debug_assertions)]
    let router = router.route("/__livereload", get(crate::dev_tools::livereload_handler));

    router.with_state(state)
}
