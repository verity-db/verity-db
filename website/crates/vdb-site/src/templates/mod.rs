//! Askama Templates
//!
//! Template structs for rendering HTML pages.

use askama::Template;
use askama_web::WebTemplate;

use crate::BUILD_VERSION;

/// Home page template.
#[derive(Template, WebTemplate)]
#[template(path = "home.html")]
pub struct HomeTemplate {
    pub title: String,
    pub tagline: String,
    /// Build version for cache busting static assets.
    pub v: &'static str,
}

impl HomeTemplate {
    pub fn new(title: impl Into<String>, tagline: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            tagline: tagline.into(),
            v: BUILD_VERSION,
        }
    }
}

/// Blog list page template.
#[derive(Template, WebTemplate)]
#[template(path = "blog/index.html")]
pub struct BlogListTemplate {
    pub title: String,
    pub posts: Vec<PostSummary>,
    /// Build version for cache busting static assets.
    pub v: &'static str,
}

impl BlogListTemplate {
    pub fn new(title: impl Into<String>, posts: Vec<PostSummary>) -> Self {
        Self {
            title: title.into(),
            posts,
            v: BUILD_VERSION,
        }
    }
}

/// Summary of a blog post for listing.
pub struct PostSummary {
    pub slug: String,
    pub title: String,
    pub date: String,
    pub excerpt: String,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
}

/// Individual blog post template.
#[derive(Template, WebTemplate)]
#[template(path = "blog/post.html")]
pub struct BlogPostTemplate {
    pub title: String,
    pub post_title: String,
    pub date: String,
    pub content_html: String,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    /// Build version for cache busting static assets.
    pub v: &'static str,
}

impl BlogPostTemplate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: impl Into<String>,
        post_title: impl Into<String>,
        date: impl Into<String>,
        content_html: impl Into<String>,
        author_name: Option<String>,
        author_avatar: Option<String>,
    ) -> Self {
        Self {
            title: title.into(),
            post_title: post_title.into(),
            date: date.into(),
            content_html: content_html.into(),
            author_name,
            author_avatar,
            v: BUILD_VERSION,
        }
    }
}
