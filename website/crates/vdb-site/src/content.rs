//! Content Loading and Parsing
//!
//! Markdown content with YAML frontmatter support.

use std::{collections::HashMap, fs, path::Path};

use chrono::NaiveDate;
use gray_matter::{engine::YAML, Matter, ParsedEntity};
use pulldown_cmark::{html, Options, Parser};
use serde::Deserialize;

/// A blog post with metadata and rendered content.
#[derive(Clone, Debug)]
pub struct BlogPost {
    pub slug: String,
    pub title: String,
    pub date: NaiveDate,
    pub excerpt: String,
    pub content_html: String,
}

/// Frontmatter for blog posts.
#[derive(Deserialize)]
struct BlogFrontmatter {
    title: String,
    slug: String,
    date: String,
    excerpt: String,
}

/// Store for all content (blog posts, etc.).
#[derive(Clone, Debug, Default)]
pub struct ContentStore {
    posts: HashMap<String, BlogPost>,
    posts_sorted: Vec<String>,
}

impl ContentStore {
    /// Load all content from the filesystem.
    pub fn load() -> Self {
        let mut store = Self::default();
        store.load_blog_posts();
        store
    }

    fn load_blog_posts(&mut self) {
        let blog_dir = Path::new("content/blog");

        if !blog_dir.exists() {
            tracing::warn!("Blog directory does not exist: {:?}", blog_dir);
            return;
        }

        let Ok(entries) = fs::read_dir(blog_dir) else {
            tracing::error!("Failed to read blog directory");
            return;
        };

        let matter = Matter::<YAML>::new();

        for entry in entries.flatten() {
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "md") {
                if let Some(post) = Self::parse_blog_post(&path, &matter) {
                    self.posts_sorted.push(post.slug.clone());
                    self.posts.insert(post.slug.clone(), post);
                }
            }
        }

        // Sort by date descending
        self.posts_sorted.sort_by(|a, b| {
            let post_a = self.posts.get(a);
            let post_b = self.posts.get(b);
            match (post_a, post_b) {
                (Some(a), Some(b)) => b.date.cmp(&a.date),
                _ => std::cmp::Ordering::Equal,
            }
        });
    }

    fn parse_blog_post(path: &Path, matter: &Matter<YAML>) -> Option<BlogPost> {
        let content = fs::read_to_string(path).ok()?;
        let parsed: ParsedEntity<BlogFrontmatter> = matter.parse(&content).ok()?;

        let frontmatter = parsed.data?;

        let date = NaiveDate::parse_from_str(&frontmatter.date, "%Y-%m-%d").ok()?;

        let options = Options::all();
        let parser = Parser::new_ext(&parsed.content, options);
        let mut content_html = String::new();
        html::push_html(&mut content_html, parser);

        Some(BlogPost {
            slug: frontmatter.slug,
            title: frontmatter.title,
            date,
            excerpt: frontmatter.excerpt,
            content_html,
        })
    }

    /// Get all blog posts sorted by date (newest first).
    pub fn blog_posts(&self) -> Vec<&BlogPost> {
        self.posts_sorted.iter().filter_map(|slug| self.posts.get(slug)).collect()
    }

    /// Get a single blog post by slug.
    pub fn blog_post(&self, slug: &str) -> Option<&BlogPost> {
        self.posts.get(slug)
    }
}
