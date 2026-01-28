//! Content Loading and Parsing
//!
//! Markdown content with YAML frontmatter support and syntax highlighting.

use std::{collections::HashMap, fs, path::Path};

use chrono::NaiveDate;
use gray_matter::{engine::YAML, Matter, ParsedEntity};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::Deserialize;
use syntect::{
    html::{ClassStyle, ClassedHTMLGenerator},
    parsing::SyntaxSet,
};

/// A blog post with metadata and rendered content.
#[derive(Clone, Debug)]
pub struct BlogPost {
    pub slug: String,
    pub title: String,
    pub date: NaiveDate,
    pub excerpt: String,
    pub content_html: String,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
}

/// Frontmatter for blog posts.
#[derive(Deserialize)]
struct BlogFrontmatter {
    title: String,
    slug: String,
    date: String,
    excerpt: String,
    author_name: Option<String>,
    author_avatar: Option<String>,
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

        let content_html = render_markdown_with_highlighting(&parsed.content);

        Some(BlogPost {
            slug: frontmatter.slug,
            title: frontmatter.title,
            date,
            excerpt: frontmatter.excerpt,
            content_html,
            author_name: frontmatter.author_name,
            author_avatar: frontmatter.author_avatar,
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

/// Render markdown to HTML with syntax highlighting for code blocks.
fn render_markdown_with_highlighting(markdown: &str) -> String {
    let ss = SyntaxSet::load_defaults_newlines();

    let options = Options::all();
    let parser = Parser::new_ext(markdown, options);

    let mut html_output = String::new();
    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_block_content = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let lang_str = lang.to_string();
                        if lang_str.is_empty() {
                            None
                        } else {
                            Some(lang_str)
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
                code_block_content.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let highlighted = highlight_code(&code_block_content, code_block_lang.as_deref(), &ss);

                let lang_class = code_block_lang
                    .as_ref()
                    .map(|l| format!(" language-{l}"))
                    .unwrap_or_default();

                html_output.push_str(&format!(
                    "<pre class=\"highlight{lang_class}\"><code>{highlighted}</code></pre>"
                ));
                in_code_block = false;
                code_block_lang = None;
            }
            Event::Text(text) if in_code_block => {
                code_block_content.push_str(&text);
            }
            other => {
                pulldown_cmark::html::push_html(&mut html_output, std::iter::once(other));
            }
        }
    }

    html_output
}

/// Highlight code using syntect with CSS classes.
fn highlight_code(code: &str, lang: Option<&str>, ss: &SyntaxSet) -> String {
    let syntax = lang
        .and_then(|l| ss.find_syntax_by_token(l))
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut html_generator = ClassedHTMLGenerator::new_with_class_style(
        syntax,
        ss,
        ClassStyle::Spaced,
    );

    for line in code.lines() {
        // ClassedHTMLGenerator expects lines without trailing newlines
        let _ = html_generator.parse_html_for_line_which_includes_newline(&format!("{line}\n"));
    }

    html_generator.finalize()
}
