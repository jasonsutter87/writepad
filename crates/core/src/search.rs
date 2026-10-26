//! Search the whole vault: every note and every take of every manuscript,
//! not just the live ones, so a word one draft (or one gospel) uses and
//! another doesn't still turns up.

use crate::{manuscript, vault};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Debug)]
pub struct Hit {
    /// "note" or "take".
    pub kind: &'static str,
    /// Note path, for a note.
    pub path: String,
    /// Manuscript, block and take ids, for a take.
    pub ms: String,
    pub block: String,
    pub take: String,
    /// Note name, or block title.
    pub title: String,
    /// Manuscript title and take label, for a take.
    pub context: String,
    /// The first match with a little text either side.
    pub snippet: String,
    /// How many times the words occur in the text.
    pub count: usize,
    /// The title (or a page property) matched.
    pub in_title: bool,
    /// The words appear together, as typed.
    pub phrase: bool,
}

/// Lowercase one char to one char, so positions in the folded text are
/// positions in the original.
fn fold(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

fn find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn count(hay: &[char], needle: &[char]) -> usize {
    let mut n = 0;
    let mut at = 0;
    while let Some(i) = find(hay, needle, at) {
        n += 1;
        at = i + needle.len();
    }
    n
}

fn snippet(text: &[char], at: usize, len: usize) -> String {
    const SIDE: usize = 50;
    let start = at.saturating_sub(SIDE);
    let end = (at + len + SIDE).min(text.len());
    let mut s: String = text[start..end].iter().collect();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        s = format!("…{s}");
    }
    if end < text.len() {
        s.push('…');
    }
    s
}

struct Score {
    snippet: String,
    count: usize,
    in_title: bool,
    phrase: bool,
}

/// Score one document: None unless every word is in the title or text.
/// `heading` is what counts as its title: the title, plus a page's properties.
fn score(words: &[Vec<char>], phrase: &[char], heading: &str, text: &str) -> Option<Score> {
    let (t, x) = (fold(heading), fold(text));
    let mut total = 0;
    let mut in_title = true;
    for w in words {
        let (a, b) = (find(&t, w, 0).is_some(), count(&x, w));
        if !a && b == 0 {
            return None;
        }
        in_title &= a;
        total += b;
    }
    let orig: Vec<char> = text.chars().collect();
    let together = words.len() > 1 && (find(&t, phrase, 0).is_some() || find(&x, phrase, 0).is_some());
    let snip = match find(&x, phrase, 0).or_else(|| find(&x, &words[0], 0)) {
        Some(i) => snippet(&orig, i, words[0].len()),
        None => snippet(&orig, 0, 0),
    };
    Some(Score { snippet: snip, count: total, in_title, phrase: together })
}

/// Notes and takes containing every word of `query`, ignoring case: title
/// and property matches first, then the words as typed, then the most
/// matches, at most `limit`.
pub fn search(vault: &Path, query: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<Vec<char>> = query.split_whitespace().map(fold).collect();
    if words.is_empty() {
        return vec![];
    }
    let phrase = fold(&query.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut hits = vec![];
    for n in vault::list_notes(vault) {
        let Ok(text) = vault::read(vault, &n.path) else { continue };
        let title = n.path.trim_end_matches(".md");
        if let Some(Score { snippet, count, in_title, phrase }) = score(&words, &phrase, title, &text) {
            hits.push(Hit {
                kind: "note",
                path: n.path.clone(),
                ms: String::new(),
                block: String::new(),
                take: String::new(),
                title: title.to_string(),
                context: String::new(),
                snippet,
                count,
                in_title,
                phrase,
            });
        }
    }
    for info in manuscript::list(vault) {
        let Ok(m) = manuscript::load(vault, &info.id) else { continue };
        for b in &m.blocks {
            // A page's properties are searched with its live take.
            let props: Vec<String> = b.props.iter().map(|p| format!("{}: {}", p.key, p.value)).collect();
            for v in &b.variants {
                let live = v.id == b.live && !props.is_empty();
                let text = if live { format!("{}\n\n{}", props.join("\n"), v.text) } else { v.text.clone() };
                let heading = if live { format!("{}\n{}", b.title, props.join("\n")) } else { b.title.clone() };
                if let Some(Score { snippet, count, in_title, phrase }) = score(&words, &phrase, &heading, &text) {
                    let context = if b.variants.len() > 1 { format!("{} · {}", m.title, v.label) } else { m.title.clone() };
                    hits.push(Hit {
                        kind: "take",
                        path: String::new(),
                        ms: m.id.clone(),
                        block: b.id.clone(),
                        take: v.id.clone(),
                        title: b.title.clone(),
                        context,
                        snippet,
                        count,
                        in_title,
                        phrase,
                    });
                }
            }
        }
    }
    // Stable, so equal hits keep vault order (notes, then manuscript order).
    hits.sort_by(|a, b| b.in_title.cmp(&a.in_title).then(b.phrase.cmp(&a.phrase)).then(b.count.cmp(&a.count)));
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_words_in_notes_and_every_take() {
        let dir = std::env::temp_dir().join(format!("writepad-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let v = dir.as_path();
        std::fs::write(v.join("Legion.md"), "The demons begged him.").unwrap();
        // Drafts to import live outside the vault, or they'd be notes too.
        let src = std::env::temp_dir().join(format!("writepad-search-src-{}", std::process::id()));
        std::fs::create_dir_all(&src).unwrap();
        let a = src.join("Matthew.md");
        let b = src.join("Mark.md");
        std::fs::write(&a, "## The pigs\n\nIn the country of the Gergesenes, two met him.\n").unwrap();
        std::fs::write(&b, "## The pigs\n\nIn the country of the Gadarenes, a man met him. The demons begged.\n").unwrap();
        manuscript::import(v, "Gospels", &[a.to_string_lossy().into(), b.to_string_lossy().into()]).unwrap();

        let h = search(v, "gadarenes", 10);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].kind, "take");
        assert_eq!(h[0].context, "Gospels · Mark");
        assert!(h[0].snippet.contains("Gadarenes"));

        let h = search(v, "DEMONS begged", 10);
        let kinds: Vec<_> = h.iter().map(|h| h.kind).collect();
        assert!(kinds.contains(&"note") && kinds.contains(&"take"), "{kinds:?}");

        // A title match comes first.
        let h = search(v, "pigs", 10);
        assert!(h[0].in_title);
        assert!(search(v, "pigs gergesenes", 10).len() == 1);
        assert!(search(v, "   ", 10).is_empty());

        // Page properties are found with the live take.
        let m = manuscript::manifest(v, &h[0].ms).unwrap();
        let props = vec![manuscript::Prop { key: "verses".into(), value: "Matthew 8:28-34".into() }];
        manuscript::set_block_props(v, &m.id, &m.blocks[0].id, props).unwrap();
        let h = search(v, "8:28", 10);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].take, m.blocks[0].live);
        assert!(h[0].snippet.starts_with("verses: Matthew 8:28"), "{}", h[0].snippet);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&src).unwrap();
    }
}
