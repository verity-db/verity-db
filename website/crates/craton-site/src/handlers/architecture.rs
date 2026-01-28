//! Architecture Page Handler

use axum::response::IntoResponse;

use crate::templates::ArchitectureTemplate;

/// Handler for the architecture deep dive page.
pub async fn architecture() -> impl IntoResponse {
    ArchitectureTemplate::new("Craton")
}
