//! Router Configuration
//!
//! Route configuration for the website.

use axum::{routing::get, Router};
use tower::ServiceBuilder;
use tower_http::{
    services::ServeDir,
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::{handlers, state::AppState};

/// Create the main router with all routes.
pub fn create_router(state: AppState) -> Router {
    // Static file service with cache headers
    // In production, files are served with cache-control that requires revalidation.
    // The ?v= query parameter provides cache busting when files change.
    let static_service = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(ServeDir::new("public"));

    let router = Router::new()
        .route("/", get(handlers::home::home))
        .route("/architecture", get(handlers::architecture::architecture))
        .route("/blog", get(handlers::blog::blog_index))
        .route("/blog/{slug}", get(handlers::blog::blog_post))
        .nest_service("/public", static_service)
        .layer(TraceLayer::new_for_http());

    #[cfg(debug_assertions)]
    let router = router.route("/__livereload", get(crate::dev_tools::livereload_handler));

    router.with_state(state)
}
