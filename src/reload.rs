use std::convert::Infallible;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{Stream, StreamExt};
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::server::Config;

const CLIENT_JS: &str = include_str!("client.js");
const SCRIPT_TAG: &str = "<script src=\"/__preview/reload.js\"></script>";

/// Directories whose changes never trigger a reload.
const IGNORED_DIRS: &[&str] = &[".git", "node_modules"];

/// Watch `root` and broadcast the URL paths of changed files.
pub fn watch(
    root: &Path,
    log: bool,
) -> Result<(broadcast::Sender<Vec<String>>, Debouncer<RecommendedWatcher>), String> {
    let (tx, _) = broadcast::channel(16);
    let sender = tx.clone();
    let base = root.to_path_buf();

    let mut debouncer = new_debouncer(Duration::from_millis(100), move |res: DebounceEventResult| {
        let events = match res {
            Ok(events) => events,
            Err(e) => {
                eprintln!("watch error: {e}");
                return;
            }
        };
        let mut paths: Vec<String> = events
            .iter()
            .filter_map(|e| url_path(&base, &e.path))
            .collect();
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return;
        }
        if log {
            println!("changed: {}", paths.join(", "));
        }
        // An error only means no browser is connected right now.
        let _ = sender.send(paths);
    })
    .map_err(|e| format!("cannot start file watcher: {e}"))?;

    debouncer
        .watcher()
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| format!("cannot watch {}: {e}", root.display()))?;

    Ok((tx, debouncer))
}

/// Convert a changed file to the URL path it is served at, or `None` if it should be ignored.
fn url_path(root: &Path, path: &Path) -> Option<String> {
    // Some platforms report paths through symlinks; fall back to the raw path.
    let rel = path
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let real: PathBuf = path.canonicalize().ok()?;
            real.strip_prefix(root).ok().map(Path::to_path_buf)
        })?;

    let mut parts = Vec::new();
    for c in rel.components() {
        let Component::Normal(name) = c else { continue };
        let name = name.to_str()?;
        if IGNORED_DIRS.contains(&name) {
            return None;
        }
        parts.push(name);
    }
    let file = *parts.last()?;
    if is_editor_temp(file) {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

fn is_editor_temp(name: &str) -> bool {
    name.ends_with('~')
        || name.starts_with(".#")
        // BSD/macOS `sed -i` and JetBrains IDEs save through temp files.
        || name.starts_with(".!")
        || name.contains("___jb_")
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name == "4913"
        || name == ".DS_Store"
}

/// Server-sent events stream of changed paths, one `change` event per batch.
pub async fn events(
    State(cfg): State<std::sync::Arc<Config>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = match &cfg.reload {
        Some(tx) => tx.subscribe(),
        // Reload disabled: a stream that never emits.
        None => broadcast::channel(1).1,
    };
    let stream = BroadcastStream::new(rx).filter_map(|msg| async move {
        let paths = msg.ok()?;
        let data = serde_json::to_string(&paths).ok()?;
        Some(Ok(Event::default().event("change").data(data)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn client_script() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], CLIENT_JS)
}

/// Insert the reload script before the last `</body>`, or append it.
pub fn inject_script(html: &mut String) {
    // ASCII lowercasing keeps byte offsets identical to the original.
    match html.to_ascii_lowercase().rfind("</body>") {
        Some(i) => html.insert_str(i, SCRIPT_TAG),
        None => html.push_str(SCRIPT_TAG),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_before_body() {
        let mut html = "<html><BODY>hi</BODY></html>".to_string();
        inject_script(&mut html);
        assert_eq!(html, format!("<html><BODY>hi{SCRIPT_TAG}</BODY></html>"));

        let mut fragment = "<p>hi</p>".to_string();
        inject_script(&mut fragment);
        assert_eq!(fragment, format!("<p>hi</p>{SCRIPT_TAG}"));
    }

    #[test]
    fn maps_and_filters_paths() {
        let root = Path::new("/site");
        assert_eq!(url_path(root, Path::new("/site/css/a.css")).as_deref(), Some("/css/a.css"));
        assert_eq!(url_path(root, Path::new("/site/.git/index")), None);
        assert_eq!(url_path(root, Path::new("/site/node_modules/x/y.js")), None);
        assert_eq!(url_path(root, Path::new("/site/index.html~")), None);
        assert_eq!(url_path(root, Path::new("/site/.index.html.swp")), None);
        assert_eq!(url_path(root, Path::new("/site/css/.!48301!style.css")), None);
        assert_eq!(url_path(root, Path::new("/site/a.html___jb_tmp___")), None);
    }
}
