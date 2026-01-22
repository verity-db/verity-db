//! Home Page Handler

use axum::response::IntoResponse;

use crate::templates::HomeTemplate;

/// Handler for the landing page.
pub async fn home() -> impl IntoResponse {
    HomeTemplate {
        title: "VerityDB".to_string(),
        tagline: "The Verifiable Database for Regulated Industries".to_string(),
    }
}
