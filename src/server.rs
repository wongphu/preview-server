use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use percent_encoding::percent_decode_str;
use tokio::sync::broadcast;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::{listing, reload};

/// URL prefix reserved for the server's own endpoints.
pub const INTERNAL_PREFIX: &str = "/__preview/";

const INDEX_FILES: &[&str] = &["index.html", "index.htm"];

pub struct Config {
    /// Canonicalized directory being served.
    pub root: PathBuf,
    pub spa: bool,
    pub listing: bool,
    /// Present when live reload is enabled.
    pub reload: Option<broadcast::Sender<Vec<String>>>,
}

type AppState = Arc<Config>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/__preview/events", get(reload::events))
        .route("/__preview/reload.js", get(reload::client_script))
        .fallback(serve)
        .with_state(state)
        // Dev server: always revalidate so edits show up immediately.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
}

async fn serve(State(cfg): State<AppState>, req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response();
    }

    let uri = req.uri().clone();
    let Some(path) = resolve(&cfg.root, uri.path()) else {
        return not_found(&cfg, uri.path());
    };

    match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_dir() => {
            // Redirect /dir to /dir/ so relative links inside the page resolve correctly.
            if !uri.path().ends_with('/') {
                return Redirect::permanent(&with_trailing_slash(&uri)).into_response();
            }
            for name in INDEX_FILES {
                let index = path.join(name);
                if is_file(&index).await {
                    return serve_file(&cfg, &index, req).await;
                }
            }
            if cfg.listing {
                return match listing::render(&path, uri.path()).await {
                    Ok(page) => html_page(&cfg, page),
                    Err(e) => error_page(&e),
                };
            }
            not_found(&cfg, uri.path())
        }
        Ok(_) => serve_file(&cfg, &path, req).await,
        Err(_) => {
            if cfg.spa && looks_like_route(uri.path()) {
                for name in INDEX_FILES {
                    let index = cfg.root.join(name);
                    if is_file(&index).await {
                        return serve_file(&cfg, &index, req).await;
                    }
                }
            }
            not_found(&cfg, uri.path())
        }
    }
}

/// Map a URL path to a file inside `root`, rejecting anything that escapes it.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode_str(url_path).decode_utf8().ok()?;
    let mut path = root.to_path_buf();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => return None,
            s if s.contains('\\') || s.contains('\0') => return None,
            s => path.push(s),
        }
    }
    // Symlinks could still point outside the root; check the real location.
    match path.canonicalize() {
        Ok(real) if !real.starts_with(root) => None,
        _ => Some(path),
    }
}

async fn serve_file(cfg: &Config, path: &Path, req: Request) -> Response {
    if cfg.reload.is_some() && is_html(path) {
        return match tokio::fs::read(path).await {
            Ok(bytes) => html_page(cfg, String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) => error_page(&e),
        };
    }
    // ServeFile handles content types, ranges, and conditional requests.
    match ServeFile::new(path).oneshot(req).await {
        Ok(res) => res.map(Body::new),
        Err(e) => error_page(&e),
    }
}

/// An HTML response with the live-reload script injected when enabled.
fn html_page(cfg: &Config, mut html: String) -> Response {
    if cfg.reload.is_some() {
        reload::inject_script(&mut html);
    }
    Html(html).into_response()
}

fn not_found(cfg: &Config, url_path: &str) -> Response {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Not found</title>\
         <body style=\"font-family:system-ui,sans-serif;padding:2rem\">\
         <h1>404</h1><p><code>{}</code> was not found.</p></body>",
        listing::escape(url_path)
    );
    // Includes the reload script so the page refreshes once the file is created.
    let mut res = html_page(cfg, body);
    *res.status_mut() = StatusCode::NOT_FOUND;
    res
}

fn error_page(err: &dyn std::fmt::Display) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("error: {err}")).into_response()
}

async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file())
}

fn is_html(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

/// SPA fallback only applies to paths like `/users/42`, not missing assets like `/app.js`.
fn looks_like_route(url_path: &str) -> bool {
    let last = url_path.rsplit('/').next().unwrap_or("");
    !last.contains('.')
}

fn with_trailing_slash(uri: &Uri) -> String {
    match uri.query() {
        Some(q) => format!("{}/?{q}", uri.path()),
        None => format!("{}/", uri.path()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_traversal() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(resolve(&root, "/../etc/passwd").is_none());
        assert!(resolve(&root, "/a/%2e%2e/%2e%2e/etc").is_none());
        assert!(resolve(&root, "/a\\..\\b").is_none());
        assert_eq!(
            resolve(&root, "/a/./b%20c"),
            Some(root.join("a").join("b c"))
        );
    }

    #[test]
    fn route_detection() {
        assert!(looks_like_route("/users/42"));
        assert!(looks_like_route("/"));
        assert!(!looks_like_route("/app.js"));
    }
}
