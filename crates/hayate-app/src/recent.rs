//! Persisted "recent presentations" list for the home screen. Stored as a JSON array of file
//! paths under the user's data dir; missing files are filtered out on read.

use std::path::PathBuf;

const MAX_RECENTS: usize = 12;

/// Path to the recent-list JSON (`$XDG_DATA_HOME/hayateoffice/recent.json`, falling back to
/// `$HOME/.local/share/...`, then the current directory).
fn recent_file() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("hayateoffice").join("recent.json")
}

/// Load the recent paths, most-recent first, keeping only files that still exist.
pub(crate) fn load() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(recent_file()) else {
        return Vec::new();
    };
    let list: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    list.into_iter()
        .filter(|p| std::path::Path::new(p).is_file())
        .take(MAX_RECENTS)
        .collect()
}

/// Record `path` as the most recent, de-duplicating and capping the list. Best-effort: write
/// errors are ignored.
pub(crate) fn add(path: &str) {
    let mut list = load();
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(MAX_RECENTS);
    let file = recent_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(file, json);
    }
}
