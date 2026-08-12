//! Embed the built dashboard into the binary.
//!
//! Walks `frontend/dist` and writes a table of `include_bytes!` into `OUT_DIR`.
//! No crate does this for us on purpose: `rust-embed` and `include_dir` would
//! each be a direct dependency for a directory walk, and `CONTRIBUTING.md` asks
//! what a new one does that the standard library cannot. This is forty lines.
//!
//! When `frontend/dist` is absent the tables are emitted empty and the
//! `dashboard_assets` cfg is not set, so the crate still compiles and
//! `cargo install --path crates/doppel-cli` works on a machine with no Node.
//! `GET /` then answers 503 naming the command that builds it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    // `cargo::` rather than `cargo:` -- the older single-colon form is
    // deprecated, and `rustc-check-cfg` is what stops the `unexpected_cfgs`
    // lint from firing on our own conditional.
    println!("cargo::rustc-check-cfg=cfg(dashboard_assets)");

    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../frontend/dist")
        .canonicalize()
        .ok()
        .filter(|dist| dist.join("index.html").is_file());

    // Rebuilt when the dashboard is rebuilt. Watching the directory covers a
    // file appearing or changing inside it; watching the parent covers `dist`
    // itself appearing for the first time, which is the case a first
    // `npm run build` produces.
    println!("cargo::rerun-if-changed=../../frontend/dist");
    println!("cargo::rerun-if-changed=../../frontend");

    let mut generated = String::new();
    let Some(dist) = dist else {
        writeln!(
            generated,
            "/// No `frontend/dist` was present when this binary was built.\n\
             pub static ASSETS: &[(&str, &[u8], &str)] = &[];\n\
             pub static INDEX_HTML: &str = \"\";"
        )
        .expect("writing to a String cannot fail");
        write(&generated);
        return;
    };

    println!("cargo::rustc-cfg=dashboard_assets");

    let mut files = Vec::new();
    collect(&dist, &dist, &mut files);
    files.sort();

    writeln!(
        generated,
        "/// Every file of `frontend/dist` except `index.html`, as\n\
         /// (request path under `/static/`, bytes, content type).\n\
         pub static ASSETS: &[(&str, &[u8], &str)] = &["
    )
    .expect("writing to a String cannot fail");
    for (request_path, absolute) in &files {
        writeln!(
            generated,
            "    ({request_path:?}, include_bytes!({absolute:?}), {:?}),",
            content_type(request_path)
        )
        .expect("writing to a String cannot fail");
    }
    writeln!(generated, "];").expect("writing to a String cannot fail");

    // The page has to carry the element the listener substitutes the runtime
    // configuration into. Checked here rather than at request time: it is a
    // property of the build, so a mismatch between `frontend/index.html` and
    // `dashboard.rs` should stop the build rather than serve a page whose
    // configuration is the development placeholder.
    let index = dist.join("index.html");
    let markup = std::fs::read_to_string(&index).expect("frontend/dist/index.html is readable");
    assert!(
        markup.contains("id=\"doppel-config\""),
        "{} has no doppel-config element; frontend/index.html and \
         crates/doppel-admin/src/dashboard.rs have diverged",
        index.display()
    );

    writeln!(
        generated,
        "/// The page itself, with the configuration placeholder still in it.\n\
         pub static INDEX_HTML: &str = include_str!({:?});",
        index
    )
    .expect("writing to a String cannot fail");

    write(&generated);
}

fn write(generated: &str) {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("assets.rs"), generated).expect("write the asset table");
}

/// Every file under `dir`, as (path relative to `root`, absolute path).
///
/// `index.html` is skipped: it is served from `/` with the configuration
/// substituted into it, not from `/static/` verbatim, so having it in both
/// places would mean two ways to reach one page and only one of them configured.
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = std::fs::read_dir(dir).expect("frontend/dist is readable");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("walked below the root")
            .to_string_lossy()
            // A request path uses forward slashes whatever the build host does.
            .replace('\\', "/");
        if relative == "index.html" {
            continue;
        }
        out.push((relative, path));
    }
}

/// The content type for a built asset, by extension.
///
/// Only the extensions vite emits. Anything else is served as bytes rather than
/// guessed at: a wrong `Content-Type` on a script is a page that silently does
/// not run.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}
