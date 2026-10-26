//! Search the whole vault: every note and every take of every manuscript,
//! not just the live ones, so a word one draft (or one gospel) uses and
//! another doesn't still turns up. A query that reads as a reference
//! ("Matthew 1:1-2", "Luke 15") also finds pages whose properties cite a
//! range that overlaps it.

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
    /// A page property cites a range overlapping the reference searched for.
    pub reference: bool,
}

/// (chapter, verse); a whole chapter runs from verse 0 to `u32::MAX`.
type Pos = (u32, u32);

#[derive(Debug, PartialEq)]
struct Ref {
    /// Lowercase, single-spaced, without a trailing period: "matthew", "1 john".
    book: String,
    from: Pos,
    to: Pos,
}

impl Ref {
    fn overlaps(&self, other: &Ref) -> bool {
        let (a, b) = (&self.book, &other.book);
        let same = a.is_empty() || b.is_empty() || a == b || (a.len() >= 3 && b.starts_with(a.as_str())) || (b.len() >= 3 && a.starts_with(b.as_str()));
        same && self.from <= other.to && other.from <= self.to
    }
}

fn dash(c: char) -> bool {
    matches!(c, '-' | '–' | '—' | '‒' | '−')
}

/// `c`, `c:v`, `c-c2`, `c:v-v2` or `c:v-c2:v2`, with any trailing text ignored.
fn range(s: &str) -> Option<(Pos, Pos)> {
    fn num(s: &str) -> Option<(u32, &str)> {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        Some((s[..end].parse().ok()?, &s[end..]))
    }
    fn point(s: &str) -> Option<(u32, Option<u32>, &str)> {
        let (c, rest) = num(s)?;
        match rest.strip_prefix(':').or(rest.strip_prefix('.')).and_then(num) {
            Some((v, rest)) => Some((c, Some(v), rest)),
            None => Some((c, None, rest)),
        }
    }
    let (c, v, rest) = point(s)?;
    let from = (c, v.unwrap_or(0));
    let mut to = (c, v.unwrap_or(u32::MAX));
    let rest = rest.trim_start();
    if let Some(r) = rest.strip_prefix(dash).map(str::trim_start) {
        if let Some((c2, v2, _)) = point(r) {
            to = match (v, v2) {
                (_, Some(v2)) => (c2, v2),               // 1:18-2:3, 1-2:3
                (Some(_), None) => (c, c2),              // 1:1-17: a verse
                (None, None) => (c2, u32::MAX),          // 1-3: chapters
            };
        }
    }
    (from <= to).then_some((from, to))
}

/// Every reference in a property value: "Matthew 1:1–17; Luke 3:23–38",
/// "Mark 1:1-8, 2:3" (the book carries over), "1 John 3:16".
fn refs(text: &str) -> Vec<Ref> {
    let mut out = vec![];
    let mut book = String::new();
    for part in text.split([';', ',', '\n']) {
        let part = part.trim();
        // The reference starts at the first number, unless that number opens
        // the book's name ("1 John 3:16").
        let words: Vec<&str> = part.split_whitespace().collect();
        let Some(i) = (0..words.len()).find(|&i| {
            let opens_name = i == 0 && words.get(1).is_some_and(|w| w.starts_with(char::is_alphabetic));
            words[i].starts_with(|c: char| c.is_ascii_digit()) && !opens_name
        }) else {
            continue;
        };
        let name = words[..i].join(" ").to_lowercase().trim_end_matches('.').to_string();
        if !name.is_empty() {
            book = name;
        }
        if let Some((from, to)) = range(&words[i..].join(" ")) {
            out.push(Ref { book: book.clone(), from, to });
        }
    }
    out
}

/// The query as one reference, if that's all it is.
fn query_ref(q: &str) -> Option<Ref> {
    let r = refs(q);
    let [only] = r.as_slice() else { return None };
    // "Luke 15" yes; "chapter 15 of my draft" no.
    let rest = q.split_whitespace().last()?;
    rest.starts_with(|c: char| c.is_ascii_digit()).then(|| Ref { book: only.book.clone(), from: only.from, to: only.to })
}

/// Lowercase one char to one char, so positions in the folded text are
/// positions in the original.
fn fold(s: &str) -> Vec<char> {
    s.chars().map(|c| if dash(c) { '-' } else { c.to_lowercase().next().unwrap_or(c) }).collect()
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
    let wanted = query_ref(query);
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
                reference: false,
            });
        }
    }
    for info in manuscript::list(vault) {
        let Ok(m) = manuscript::load(vault, &info.id) else { continue };
        for b in &m.blocks {
            // A page's properties are searched with its live take.
            let props: Vec<String> = b.props.iter().map(|p| format!("{}: {}", p.key, p.value)).collect();
            // The property citing the reference searched for, if any.
            let cites = wanted.as_ref().and_then(|w| props.iter().zip(&b.props).find(|(_, p)| refs(&p.value).iter().any(|r| r.overlaps(w))).map(|(line, _)| line));
            if let Some(line) = cites {
                let v = b.variants.iter().find(|v| v.id == b.live).unwrap_or(&b.variants[0]);
                hits.push(Hit {
                    kind: "take",
                    path: String::new(),
                    ms: m.id.clone(),
                    block: b.id.clone(),
                    take: v.id.clone(),
                    title: b.title.clone(),
                    context: if b.variants.len() > 1 { format!("{} · {}", m.title, v.label) } else { m.title.clone() },
                    snippet: line.clone(),
                    count: 1,
                    in_title: true,
                    phrase: true,
                    reference: true,
                });
            }
            for v in &b.variants {
                if cites.is_some() && v.id == b.live {
                    continue; // already found, by its reference
                }
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
                        reference: false,
                    });
                }
            }
        }
    }
    // Stable, so equal hits keep vault order (notes, then manuscript order).
    hits.sort_by(|a, b| {
        (b.reference, b.in_title, b.phrase, b.count).cmp(&(a.reference, a.in_title, a.phrase, a.count))
    });
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

    #[test]
    fn reads_references() {
        let r = |book: &str, from: Pos, to: Pos| Ref { book: book.into(), from, to };
        const END: u32 = u32::MAX;
        assert_eq!(refs("Matthew 1:1–17; Luke 3:23–38"), vec![r("matthew", (1, 1), (1, 17)), r("luke", (3, 23), (3, 38))]);
        assert_eq!(refs("Mark 1:1-8, 2:3"), vec![r("mark", (1, 1), (1, 8)), r("mark", (2, 3), (2, 3))]);
        assert_eq!(refs("1 John 3:16"), vec![r("1 john", (3, 16), (3, 16))]);
        assert_eq!(refs("John 1:18–2:3"), vec![r("john", (1, 18), (2, 3))]);
        assert_eq!(refs("Luke 15"), vec![r("luke", (15, 0), (15, END))]);
        assert_eq!(refs("Gen. 1-3"), vec![r("gen", (1, 0), (3, END))]);
        assert_eq!(refs("Matthew 1:1 – 17 (the genealogy)"), vec![r("matthew", (1, 1), (1, 17))]);
        assert!(refs("POV: June").is_empty());

        let q = |s: &str| query_ref(s).unwrap();
        let page = &refs("Matthew 1:1–17; Luke 3:23–38");
        assert!(page.iter().any(|p| p.overlaps(&q("Matthew 1:1-2"))));
        assert!(page.iter().any(|p| p.overlaps(&q("matt 1:5"))));
        assert!(page.iter().any(|p| p.overlaps(&q("Matthew 1"))));
        assert!(page.iter().any(|p| p.overlaps(&q("luke 3:38"))));
        assert!(!page.iter().any(|p| p.overlaps(&q("Matthew 1:18"))));
        assert!(!page.iter().any(|p| p.overlaps(&q("Mark 1:1"))));
        assert!(query_ref("the 3 wise men").is_none());
        assert!(query_ref("gadarenes").is_none());
    }

}
