//! Version history: snapshots are git commits in the vault's own repository,
//! so they work with any git tool (and can be pushed for backup or sync).
//!
//! Nothing here rewrites history. Restoring a block or a note writes the old
//! text back as a new change; rolling the whole vault back first snapshots
//! the present, then commits the restored files as a new snapshot.

use git2::{IndexAddOption, ObjectType, Oid, Repository, Signature, Sort};
use serde::Serialize;
use std::path::Path;

const AUTOSAVE: &str = "Autosave";

#[derive(Serialize)]
pub struct Status {
    pub enabled: bool,
    pub branch: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub id: String,
    pub message: String,
    /// Unix seconds.
    pub time: i64,
    pub auto: bool,
}

/// One past version of a document, with its text at that point.
#[derive(Serialize)]
pub struct Entry {
    pub snapshot: Snapshot,
    pub text: String,
    /// For manuscript blocks: which version's text this is.
    pub label: Option<String>,
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}

fn open(vault: &Path) -> Result<Repository, String> {
    Repository::open(vault).map_err(|_| "History is off for this vault. Turn it on first.".to_string())
}

fn signature(repo: &Repository) -> Signature<'static> {
    repo.signature()
        .map(|s| s.to_owned())
        .or_else(|_| Signature::now("WRITEpad", "writepad@localhost"))
        .expect("a fixed signature is always valid")
}

fn to_snapshot(c: &git2::Commit) -> Snapshot {
    let message = c.summary().ok().flatten().unwrap_or("").to_string();
    Snapshot { id: c.id().to_string(), auto: message == AUTOSAVE, message, time: c.time().seconds() }
}

pub fn status(vault: &Path) -> Status {
    match Repository::open(vault) {
        Ok(repo) => Status {
            enabled: true,
            branch: repo.head().ok().and_then(|h| h.shorthand().map(String::from).ok()),
        },
        Err(_) => Status { enabled: false, branch: None },
    }
}

/// Make the vault a git repository and take the first snapshot.
pub fn enable(vault: &Path) -> Result<(), String> {
    if Repository::open(vault).is_err() {
        Repository::init(vault).map_err(err)?;
        let ignore = vault.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, ".DS_Store\n*.json.tmp\n").map_err(|e| e.to_string())?;
        }
    }
    snapshot(vault, "Turned on history")?;
    Ok(())
}

/// Commit everything that changed. Returns None when nothing did.
pub fn snapshot(vault: &Path, message: &str) -> Result<Option<Snapshot>, String> {
    // One snapshot at a time across every process using this vault.
    let _lock = crate::manuscript::Lock::take(&vault.join(".writepad").join(".history.lock"))?;
    let repo = open(vault)?;
    let mut index = repo.index().map_err(err)?;
    // Never snapshot OS junk or half-written saves, even without a .gitignore.
    let mut skip_junk = |path: &Path, _: &[u8]| -> i32 {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        i32::from(name == ".DS_Store" || name.ends_with(".lock") || name.contains(".tmp-") || name.ends_with(".json.tmp"))
    };
    index.add_all(["*"], IndexAddOption::DEFAULT, Some(&mut skip_junk)).map_err(err)?;
    index.update_all(["*"], None).map_err(err)?; // picks up deletions
    index.write().map_err(err)?;
    let tree_id = index.write_tree().map_err(err)?;

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
        return Ok(None);
    }
    let tree = repo.find_tree(tree_id).map_err(err)?;
    let sig = signature(&repo);
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let message = if message.trim().is_empty() { AUTOSAVE } else { message.trim() };
    let id = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents).map_err(err)?;
    let commit = repo.find_commit(id).map_err(err)?;
    let snap = to_snapshot(&commit);
    Ok(Some(snap))
}

pub fn log(vault: &Path, limit: usize) -> Result<Vec<Snapshot>, String> {
    let repo = open(vault)?;
    let mut walk = repo.revwalk().map_err(err)?;
    if walk.push_head().is_err() {
        return Ok(vec![]); // no commits yet
    }
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;
    Ok(walk
        .filter_map(Result::ok)
        .filter_map(|id| repo.find_commit(id).ok())
        .take(limit)
        .map(|c| to_snapshot(&c))
        .collect())
}

fn blob_at(repo: &Repository, commit: &git2::Commit, rel: &str) -> Option<(Oid, String)> {
    let entry = commit.tree().ok()?.get_path(Path::new(rel)).ok()?;
    if entry.kind() != Some(ObjectType::Blob) {
        return None;
    }
    let blob = repo.find_blob(entry.id()).ok()?;
    Some((entry.id(), String::from_utf8_lossy(blob.content()).into_owned()))
}

/// Past versions of one file, newest first, skipping snapshots where it didn't
/// change. `extract` turns the file's contents into the text to show (for
/// manuscripts, one block's text); entries whose text didn't change are dropped.
fn file_history(
    vault: &Path,
    rel: &str,
    limit: usize,
    extract: impl Fn(&str) -> Option<(String, Option<String>)>,
) -> Result<Vec<Entry>, String> {
    let repo = open(vault)?;
    let mut walk = repo.revwalk().map_err(err)?;
    if walk.push_head().is_err() {
        return Ok(vec![]);
    }
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;

    let mut out: Vec<Entry> = vec![];
    let mut last_blob: Option<Oid> = None;
    for id in walk.filter_map(Result::ok) {
        let Ok(commit) = repo.find_commit(id) else { continue };
        let Some((blob, contents)) = blob_at(&repo, &commit, rel) else { break };
        if last_blob == Some(blob) {
            // Same file as the newer snapshot: credit the older one instead.
            if let Some(e) = out.last_mut() {
                e.snapshot = to_snapshot(&commit);
            }
            continue;
        }
        last_blob = Some(blob);
        let Some((text, label)) = extract(&contents) else { continue };
        match out.last_mut() {
            Some(e) if e.text == text => e.snapshot = to_snapshot(&commit),
            _ => {
                if out.len() >= limit {
                    break;
                }
                out.push(Entry { snapshot: to_snapshot(&commit), text, label });
            }
        }
    }
    Ok(out)
}

pub fn note_history(vault: &Path, rel: &str, limit: usize) -> Result<Vec<Entry>, String> {
    file_history(vault, rel, limit, |s| Some((s.to_string(), None)))
}

/// Past texts of one take (its own file, so other takes' edits don't show up).
pub fn take_history(vault: &Path, ms: &str, take: &str, limit: usize) -> Result<Vec<Entry>, String> {
    file_history(vault, &crate::manuscript::take_rel_path(ms, take), limit, |s| Some((s.to_string(), None)))
}

/// Put every tracked file back the way it was at `id`. The present is
/// snapshotted first, and the result is committed as a new snapshot.
pub fn restore_all(vault: &Path, id: &str) -> Result<(), String> {
    let repo = open(vault)?;
    let target = repo.find_commit(Oid::from_str(id).map_err(err)?).map_err(err)?;
    let when = target.summary().ok().flatten().unwrap_or("").to_string();
    snapshot(vault, "Before rolling back")?;
    let mut opts = git2::build::CheckoutBuilder::new();
    opts.force();
    repo.checkout_tree(target.as_object(), Some(&mut opts)).map_err(err)?;
    snapshot(vault, &format!("Rolled back to “{when}” ({})", &id[..7.min(id.len())]))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_history_and_rollback() {
        let dir = std::env::temp_dir().join(format!("writepad-hist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "one").unwrap();
        enable(&dir).unwrap();
        assert!(snapshot(&dir, "").unwrap().is_none(), "nothing changed");

        std::fs::write(dir.join("a.md"), "two").unwrap();
        snapshot(&dir, "").unwrap().unwrap();
        std::fs::write(dir.join("a.md"), "three").unwrap();
        snapshot(&dir, "Draft three").unwrap().unwrap();

        let h = note_history(&dir, "a.md", 10).unwrap();
        let texts: Vec<&str> = h.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["three", "two", "one"]);

        restore_all(&dir, &h[2].snapshot.id).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.md")).unwrap(), "one");
        assert_eq!(log(&dir, 10).unwrap()[0].message.starts_with("Rolled back"), true);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
