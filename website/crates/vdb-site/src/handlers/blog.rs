//! Blog Handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    state::AppState,
    templates::{BlogListTemplate, BlogPostTemplate, PostSummary},
};

/// Handler for the blog index page.
pub async fn blog_index(State(state): State<AppState>) -> impl IntoResponse {
    let posts: Vec<PostSummary> = state
        .content()
        .blog_posts()
        .into_iter()
        .map(|p| PostSummary {
            slug: p.slug.clone(),
            title: p.title.clone(),
            date: p.date.format("%B %d, %Y").to_string(),
            excerpt: p.excerpt.clone(),
            author_name: p.author_name.clone(),
            author_avatar: p.author_avatar.clone(),
        })
        .collect();

    BlogListTemplate::new("Blog - VerityDB", posts)
}

/// Handler for individual blog posts.
pub async fn blog_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let post = state.content().blog_post(&slug).ok_or(StatusCode::NOT_FOUND)?;

    Ok(BlogPostTemplate::new(
        format!("{} - VerityDB", post.title),
        post.title.clone(),
        post.date.format("%B %d, %Y").to_string(),
        post.content_html.clone(),
        post.author_name.clone(),
        post.author_avatar.clone(),
    ))
}
