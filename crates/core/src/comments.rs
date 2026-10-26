//! Comments on a manuscript's text: notes in the margin, never in the text.
//!
//!   .writepad/manuscripts/<id>/comments.json   every comment, on any take
//!
//! A comment belongs to one take and remembers the words it's on (`quote`)
//! and where they were (`from`..`to`, in UTF-16 units, as the editor counts).
//! The app keeps the offsets current as the text changes; if they drift, the
//! quote finds the words again.

use crate::manuscript::{self, Lock};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    pub take: String,
    /// The words commented on, as they read when last saved.
    pub quote: String,
    pub from: usize,
    pub to: usize,
    pub text: String,
    /// Who wrote it: empty for the author, or an agent's name.
    #[serde(default)]
    pub author: String,
    pub created: u64,
    #[serde(default)]
    pub resolved: bool,
}

fn path(vault: &Path, ms: &str) -> Result<PathBuf, String> {
    Ok(manuscript::ms_dir(vault, ms)?.join("comments.json"))
}

/// Every comment in a manuscript, oldest first.
pub fn list(vault: &Path, ms: &str) -> Result<Vec<Comment>, String> {
    match fs::read_to_string(path(vault, ms)?) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("comments.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

fn update<T>(vault: &Path, ms: &str, f: impl FnOnce(&mut Vec<Comment>) -> Result<T, String>) -> Result<T, String> {
    let file = path(vault, ms)?;
    let _lock = Lock::take(&file.with_file_name(".comments.lock"))?;
    let mut all = list(vault, ms)?;
    let out = f(&mut all)?;
    all.sort_by_key(|c| c.created);
    manuscript::write_atomic(&file, serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?.as_bytes())?;
    Ok(out)
}

/// Replace one take's comments (the app saves a take's comments together,
/// with offsets moved by the latest edits). Other takes' comments are kept.
pub fn set_for_take(vault: &Path, ms: &str, take: &str, comments: Vec<Comment>) -> Result<(), String> {
    update(vault, ms, |all| {
        all.retain(|c| c.take != take);
        all.extend(comments.into_iter().map(|c| Comment { take: take.to_string(), ..c }));
        Ok(())
    })
}

/// Comment on the first place `quote` appears in a take (for agents, which
/// point at words rather than offsets).
pub fn add(vault: &Path, ms: &str, take: &str, quote: &str, text: &str, author: &str) -> Result<Comment, String> {
    let body = manuscript::read_take(vault, ms, take)?;
    let quote = quote.trim();
    if quote.is_empty() || text.trim().is_empty() {
        return Err("A comment needs the words it's on and something to say.".into());
    }
    let at = body.find(quote).ok_or_else(|| format!("{quote:?} isn't in that take."))?;
    let from = utf16_len(&body[..at]);
    let comment = Comment {
        id: manuscript::new_id(),
        take: take.to_string(),
        quote: quote.to_string(),
        from,
        to: from + utf16_len(quote),
        text: text.trim().to_string(),
        author: author.to_string(),
        created: manuscript::now(),
        resolved: false,
    };
    let c = comment.clone();
    update(vault, ms, move |all| {
        all.push(c);
        Ok(())
    })?;
    Ok(comment)
}

/// Mark a comment resolved, or reopen it.
pub fn resolve(vault: &Path, ms: &str, id: &str, resolved: bool) -> Result<(), String> {
    update(vault, ms, |all| {
        let c = all.iter_mut().find(|c| c.id == id).ok_or("No such comment")?;
        c.resolved = resolved;
        Ok(())
    })
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_live_beside_the_text() {
        let vault = std::env::temp_dir().join(format!("writepad-comments-{}", std::process::id()));
        let _ = fs::remove_dir_all(&vault);
        fs::create_dir_all(&vault).unwrap();
        let id = manuscript::from_sections(&vault, "Book", "draft", vec![("One".into(), "“Hark at the wind,” said Mr. White.".into())]).unwrap();
        let m = manuscript::load(&vault, &id).unwrap();
        let take = m.blocks[0].live.clone();

        let c = add(&vault, &id, &take, "said Mr. White", "Who's speaking is clear already.", "line-editor").unwrap();
        // Offsets count UTF-16 units, as the editor does: the curly quotes are one each.
        assert_eq!((c.from, c.to), (20, 34));
        assert!(add(&vault, &id, &take, "not there", "x", "").is_err());

        resolve(&vault, &id, &c.id, true).unwrap();
        assert!(list(&vault, &id).unwrap()[0].resolved);

        // Saving a take's comments replaces only that take's.
        set_for_take(&vault, &id, "other", vec![Comment { take: "other".into(), ..c.clone() }]).unwrap();
        set_for_take(&vault, &id, &take, vec![]).unwrap();
        let left = list(&vault, &id).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].take, "other");
        // The text itself never changes.
        assert_eq!(manuscript::read_take(&vault, &id, &take).unwrap(), "“Hark at the wind,” said Mr. White.");
    }
}
