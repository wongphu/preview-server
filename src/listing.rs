use std::fmt::Write;
use std::path::Path;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

/// Characters to escape in a single path segment of an href.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
}

/// Render an HTML index of `dir`, which is served at `url_path` (ending in `/`).
pub async fn render(dir: &Path, url_path: &str) -> std::io::Result<String> {
    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = reader.next_entry().await? {
        // Follows symlinks so linked directories list as directories.
        let Ok(meta) = tokio::fs::metadata(entry.path()).await else {
            continue;
        };
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let title = escape(&percent_encoding::percent_decode_str(url_path).decode_utf8_lossy());
    let mut rows = String::new();
    if url_path != "/" {
        rows.push_str("<tr><td><a href=\"../\">../</a></td><td></td></tr>\n");
    }
    for e in &entries {
        let href = utf8_percent_encode(&e.name, SEGMENT).to_string();
        let (slash, size) = if e.is_dir {
            ("/", String::new())
        } else {
            ("", human_size(e.size))
        };
        let _ = writeln!(
            rows,
            "<tr><td><a href=\"{href}{slash}\">{}{slash}</a></td><td>{size}</td></tr>",
            escape(&e.name)
        );
    }

    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Index of {title}</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{ font: 15px/1.5 system-ui, sans-serif; max-width: 52rem; margin: 2rem auto; padding: 0 1rem; }}
  h1 {{ font-size: 1.25rem; font-weight: 600; }}
  table {{ width: 100%; border-collapse: collapse; }}
  td {{ padding: .3rem .5rem; border-bottom: 1px solid color-mix(in srgb, currentColor 12%, transparent); }}
  td:last-child {{ text-align: right; white-space: nowrap; opacity: .6; font-variant-numeric: tabular-nums; }}
  a {{ text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
</style>
</head>
<body>
<h1>Index of {title}</h1>
<table>
{rows}</table>
</body>
</html>
"#
    ))
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn escapes_html() {
        assert_eq!(escape("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
    }
}
