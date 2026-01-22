//! Build script for vdb-site
//!
//! In release builds, this minifies CSS using lightningcss.
//! In debug builds, this is a no-op.

fn main() {
    // Rerun if CSS changes
    println!("cargo:rerun-if-changed=../../public/css");

    // CSS minification only in release builds
    #[cfg(not(debug_assertions))]
    {
        minify_css();
    }
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
