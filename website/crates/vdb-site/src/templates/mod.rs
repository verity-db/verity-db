//! Askama Templates
//!
//! Template structs for rendering HTML pages.

use askama::Template;
use askama_web::WebTemplate;

/// Home page template.
#[derive(Template, WebTemplate)]
#[template(path = "home.html")]
pub struct HomeTemplate {
    pub title: String,
    pub tagline: String,
}

/// Blog list page template.
#[derive(Template, WebTemplate)]
#[template(path = "blog/index.html")]
pub struct BlogListTemplate {
    pub title: String,
    pub posts: Vec<PostSummary>,
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
}
