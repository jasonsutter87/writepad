//! Watch the vault and tell the frontend what changed, so takes written by
//! agents (or a sync tool, or another editor) show up while you work.

use notify::{RecursiveMode, Watcher as _};
use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct Watcher {
    _inner: notify::RecommendedWatcher,
}

/// Paths that change constantly and mean nothing to the editor.
fn noise(rel: &str) -> bool {
    rel.starts_with(".git/") || rel == ".git" || rel.ends_with(".lock") || rel.contains(".tmp-") || rel.ends_with(".DS_Store")
}

pub fn start(app: AppHandle, vault: PathBuf) -> Result<Watcher, String> {
    let (tx, rx) = channel::<notify::Result<notify::Event>>();
    let mut inner = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    inner.watch(&vault, RecursiveMode::Recursive).map_err(|e| e.to_string())?;

    // Coalesce bursts (a save is a temp write plus a rename) into one event.
    std::thread::spawn(move || {
        let mut pending: Vec<String> = vec![];
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(event)) => {
                    for p in event.paths {
                        if let Ok(rel) = p.strip_prefix(&vault) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            if !noise(&rel) && !pending.contains(&rel) {
                                pending.push(rel);
                            }
                        }
                    }
                }
                Ok(Err(_)) => {}
                Err(RecvTimeoutError::Timeout) => {
                    if !pending.is_empty() {
                        let _ = app.emit("vault-changed", std::mem::take(&mut pending));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(Watcher { _inner: inner })
}
