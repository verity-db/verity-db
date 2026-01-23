//! Home Page Handler

use axum::response::IntoResponse;

use crate::templates::HomeTemplate;

/// Handler for the landing page.
pub async fn home() -> impl IntoResponse {
    HomeTemplate {
        title: "VerityDB".to_string(),
        tagline: "VerityDB".to_string(),
    }
}
