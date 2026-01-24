//! Build script for vdb-site
//!
//! - Generates a build version hash for cache busting
//! - In release builds, minifies CSS using lightningcss

use std::process::Command;

fn main() {
    // Rerun if CSS changes
    println!("cargo:rerun-if-changed=../../public/css");
    // Rerun if git HEAD changes (new commits)
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    // Generate build version for cache busting
    generate_build_version();

    // CSS minification only in release builds
    #[cfg(not(debug_assertions))]
    {
        minify_css();
    }
}

fn generate_build_version() {
    // Try to get git commit hash, fall back to timestamp
    let version = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            // Fallback to build timestamp
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| format!("{:x}", d.as_secs()))
                .unwrap_or_else(|_| "unknown".to_string())
        });

    println!("cargo:rustc-env=BUILD_VERSION={version}");
}

#[cfg(not(debug_assertions))]
fn minify_css() {
    use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};
    use std::fs;
    use std::path::Path;

    let css_dir = Path::new("../../public/css");
    let style_path = css_dir.join("style.css");

    if !style_path.exists() {
        return;
    }

    let css = match fs::read_to_string(&style_path) {
        Ok(content) => content,
        Err(_) => return,
    };

    let stylesheet = match StyleSheet::parse(&css, ParserOptions::default()) {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut stylesheet = stylesheet;
    if stylesheet.minify(MinifyOptions::default()).is_err() {
        return;
    }

    let result = match stylesheet.to_css(PrinterOptions {
        minify: true,
        ..Default::default()
    }) {
        Ok(r) => r,
        Err(_) => return,
    };

    let output_path = css_dir.join("style.min.css");
    let _ = fs::write(output_path, result.code);
}
