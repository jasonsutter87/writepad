//! Measuring prose. Books added to the Library become numbers (never their
//! text): chapter, sentence and paragraph lengths, dialogue share, how chapters
//! end. Weighted books blend into a target, and a scene is linted against it.
//!
//! Everything here is counting; nothing is guessed.

use crate::manuscript::{self, write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Sentences longer than this count as this long in the histogram.
const SENTENCE_CAP: usize = 120;
const PARAGRAPH_CAP: usize = 600;

// ---------------------------------------------------------------- the text

/// Byte offsets into the measured text.
pub struct Sentence {
    pub start: usize,
    pub end: usize,
    pub words: usize,
    /// First word, lowercased, and where it sits.
    pub opener: String,
    pub opener_span: (usize, usize),
}

pub struct Paragraph {
    pub start: usize,
    pub end: usize,
    pub words: usize,
    pub dialogue_words: usize,
    pub sentences: Vec<Sentence>,
}

/// Words: runs of letters and digits, kept whole across apostrophes and
/// hyphens ("don't", "well-known").
pub fn count_words(s: &str) -> usize {
    word_spans(s).len()
}

fn word_spans(s: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = None;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let joiner = matches!(c, '\'' | '’' | '-') && start.is_some() && chars.peek().is_some_and(|(_, n)| n.is_alphanumeric());
        if c.is_alphanumeric() || joiner {
            start.get_or_insert(i);
        } else if let Some(st) = start.take() {
            out.push((st, i));
        }
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

/// Words inside quotation marks. Quotes reset at each paragraph, so a speech
/// that runs on without a closing mark doesn't swallow the narration after it.
/// `cap` ends a quote after that many words, for scans that lost both their
/// paragraphs and some closing marks.
fn dialogue_words(p: &str, cap: Option<usize>) -> usize {
    let mut inside = false;
    let mut in_word = false;
    let mut n = 0;
    let mut run = 0;
    for c in p.chars() {
        match c {
            '“' => (inside, run) = (true, 0),
            '”' => inside = false,
            '"' => (inside, run) = (!inside, 0),
            _ => {}
        }
        let w = c.is_alphanumeric();
        if w && !in_word && inside {
            n += 1;
            run += 1;
            if cap.is_some_and(|cap| run >= cap) {
                inside = false;
            }
        }
        in_word = w || (in_word && matches!(c, '\'' | '’' | '-'));
    }
    n
}

fn is_abbreviation(before: &str) -> bool {
    let word = before.rsplit(|c: char| c.is_whitespace() || c == '(' || c == '“' || c == '"').next().unwrap_or("");
    let w = word.trim_start_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    matches!(w.as_str(), "mr" | "mrs" | "ms" | "dr" | "st" | "jr" | "sr" | "vs" | "etc" | "e.g" | "i.e" | "prof" | "mt" | "no" | "approx")
        || (w.chars().count() == 1 && w.chars().all(|c| c.is_alphabetic()))
}

/// Split one paragraph into sentences. A stop followed by a lowercase word
/// ("Stop!" he said) doesn't end the sentence, and neither does "Mr.".
fn sentences(text: &str, base: usize) -> Vec<Sentence> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let byte = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(text.len());
    let mut out = vec![];
    let push = |from: usize, to: usize, out: &mut Vec<Sentence>| {
        let s = &text[from..to];
        let spans = word_spans(s);
        if let Some(&(a, b)) = spans.first() {
            out.push(Sentence {
                start: base + from,
                end: base + to,
                words: spans.len(),
                opener: s[a..b].to_lowercase(),
                opener_span: (base + from + a, base + from + b),
            });
        }
    };
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        if !matches!(c, '.' | '!' | '?' | '…') {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?' | '…' | '"' | '”' | '’' | '\'' | ')' | ']' | '*' | '_') {
            j += 1;
        }
        if j < chars.len() && !chars[j].1.is_whitespace() {
            i = j;
            continue;
        }
        let mut k = j;
        while k < chars.len() && chars[k].1.is_whitespace() {
            k += 1;
        }
        let lower_next = chars.get(k).is_some_and(|n| n.1.is_lowercase());
        let abbrev = c == '.' && j == i + 1 && is_abbreviation(&text[byte(start)..byte(i)]);
        if k >= chars.len() || (!lower_next && !abbrev) {
            push(byte(start), byte(j), &mut out);
            start = k;
        }
        i = k.max(i + 1);
    }
    if start < chars.len() {
        push(byte(start), text.len(), &mut out);
    }
    out
}

/// Lines that aren't prose: headings, rules, scene breaks, and gamebook
/// choices ("If you run, turn to 12.").
fn is_markup_line(line: &str) -> bool {
    let t = line.trim();
    let choice = t.to_lowercase().find("turn to ").is_some_and(|i| t[i + 8..].starts_with(|c: char| c.is_ascii_digit()));
    t.starts_with('#') || t.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ' | '=')) || t == "⁂" || choice
}

/// Paragraphs (blank-line separated) with their sentences, skipping
/// frontmatter, headings and scene breaks.
pub fn measure(text: &str) -> Vec<Paragraph> {
    measure_with(text, None)
}

fn measure_with(text: &str, quote_cap: Option<usize>) -> Vec<Paragraph> {
    let body_start = frontmatter_end(text);
    let mut out = vec![];
    let mut para: Option<(usize, usize)> = None;
    let mut offset = 0;
    let close = |p: Option<(usize, usize)>, out: &mut Vec<Paragraph>| {
        if let Some((a, b)) = p {
            let s = &text[a..b];
            let words = count_words(s);
            if words > 0 {
                out.push(Paragraph { start: a, end: b, words, dialogue_words: dialogue_words(s, quote_cap), sentences: sentences(s, a) });
            }
        }
    };
    for line in text.split_inclusive('\n') {
        let (a, b) = (offset, offset + line.trim_end().len());
        offset += line.len();
        if a < body_start {
            continue;
        }
        if line.trim().is_empty() || is_markup_line(line) {
            close(para.take(), &mut out);
        } else {
            para = Some(match para {
                Some((s, _)) => (s, b),
                None => (a, b),
            });
        }
    }
    close(para, &mut out);
    out
}

fn frontmatter_end(text: &str) -> usize {
    if !text.starts_with("---\n") {
        return 0;
    }
    text[4..].find("\n---").map(|i| {
        let after = 4 + i + 4;
        text[after..].find('\n').map(|n| after + n + 1).unwrap_or(text.len())
    }).unwrap_or(0)
}

fn syllables(word: &str) -> usize {
    let w: Vec<char> = word.to_lowercase().chars().filter(|c| c.is_alphabetic()).collect();
    if w.is_empty() {
        return 0;
    }
    let vowel = |c: char| "aeiouy".contains(c);
    let mut n = 0;
    let mut prev = false;
    for &c in &w {
        let v = vowel(c);
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    if w.len() > 2 && w[w.len() - 1] == 'e' && !vowel(w[w.len() - 2]) && !(w[w.len() - 2] == 'l' && w.len() > 3) {
        n -= 1;
    }
    n.max(1)
}

const NOT_ADVERBS: &[&str] = &[
    "only", "family", "early", "likely", "holy", "reply", "supply", "apply", "belly", "bully", "fly", "july", "italy", "ally", "rally",
    "jelly", "silly", "ugly", "lonely", "lovely", "friendly", "daily", "elderly", "costly", "curly", "lily", "sally", "molly", "emily",
    "kelly", "holly", "billy", "willy", "assembly", "anomaly", "rely", "imply", "comply", "multiply", "butterfly", "deadly", "lively",
    "chilly", "hilly", "jolly", "oily", "smelly", "wobbly", "bubbly", "cuddly", "orderly", "scholarly", "ghostly", "manly", "womanly",
];

fn is_adverb(word: &str) -> bool {
    let w = word.to_lowercase();
    w.len() > 4 && w.ends_with("ly") && !NOT_ADVERBS.contains(&w.as_str())
}

// ---------------------------------------------------------------- profiles

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ChapterStat {
    pub title: String,
    pub words: u32,
    pub sentences: u32,
    pub dialogue: f32,
    /// dialogue | question | short | other
    pub ending: String,
}

/// Everything measured about one book. No text is kept.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Profile {
    pub id: String,
    pub title: String,
    pub author: String,
    pub source: String,
    pub measured: u64,
    pub chapters: Vec<ChapterStat>,
    /// Count of sentences by length in words (index = words, capped).
    pub sentence_hist: Vec<u32>,
    pub paragraph_hist: Vec<u32>,
    pub words: u64,
    pub sentences: u64,
    pub paragraphs: u64,
    pub dialogue_words: u64,
    pub one_sentence_paras: u64,
    pub adverbs: u64,
    pub syllables: u64,
    /// Sentences that open with the same word as the two before them.
    pub repeated_openers: u64,
    /// The source lost its paragraph breaks (a scan), so paragraph numbers are unknown.
    #[serde(default)]
    pub no_paragraphs: bool,
}

fn ending_of(paras: &[Paragraph], text: &str) -> &'static str {
    let Some(last) = paras.last() else { return "other" };
    let s = text[last.start..last.end].trim_end();
    if last.dialogue_words * 2 > last.words || s.ends_with('”') || s.ends_with('"') {
        return "dialogue";
    }
    let Some(sent) = last.sentences.last() else { return "other" };
    if text[sent.start..sent.end].trim_end_matches(|c: char| !c.is_alphanumeric() && c != '?').ends_with('?') {
        "question"
    } else if sent.words <= 6 {
        "short"
    } else {
        "other"
    }
}

impl Profile {
    pub fn new(id: &str, title: &str, author: &str, source: &str) -> Profile {
        Profile {
            id: id.into(),
            title: title.into(),
            author: author.into(),
            source: source.into(),
            measured: manuscript::now(),
            sentence_hist: vec![0; SENTENCE_CAP + 1],
            paragraph_hist: vec![0; PARAGRAPH_CAP + 1],
            ..Default::default()
        }
    }

    pub fn add_chapter(&mut self, title: &str, text: &str) {
        let paras = measure_with(text, self.no_paragraphs.then_some(60));
        let mut words = 0;
        let mut sentence_count = 0;
        let mut dialogue = 0;
        let mut openers: Vec<&str> = vec![];
        for p in &paras {
            words += p.words;
            dialogue += p.dialogue_words;
            sentence_count += p.sentences.len();
            if !self.no_paragraphs {
                self.paragraph_hist[p.words.min(PARAGRAPH_CAP)] += 1;
                if p.sentences.len() == 1 {
                    self.one_sentence_paras += 1;
                }
            }
            for s in &p.sentences {
                self.sentence_hist[s.words.min(SENTENCE_CAP)] += 1;
                let n = openers.len();
                if n >= 2 && openers[n - 1] == s.opener && openers[n - 2] == s.opener {
                    self.repeated_openers += 1;
                }
                openers.push(&s.opener);
            }
            for (a, b) in word_spans(&text[p.start..p.end]) {
                let w = &text[p.start + a..p.start + b];
                self.syllables += syllables(w) as u64;
                if is_adverb(w) {
                    self.adverbs += 1;
                }
            }
        }
        self.words += words as u64;
        self.sentences += sentence_count as u64;
        self.paragraphs += paras.len() as u64;
        self.dialogue_words += dialogue as u64;
        self.chapters.push(ChapterStat {
            title: title.into(),
            words: words as u32,
            sentences: sentence_count as u32,
            dialogue: if words > 0 { dialogue as f32 / words as f32 } else { 0.0 },
            ending: ending_of(&paras, text).into(),
        });
    }
}

/// Pages that aren't the story: skip them when measuring a book.
pub fn is_front_matter(title: &str, words: usize) -> bool {
    let t = title.to_lowercase();
    const SKIP: &[&str] = &[
        "copyright", "dedication", "contents", "acknowledg", "also by", "about the author", "praise", "title page", "cover",
        "books by", "by the same author", "newsletter", "excerpt", "sneak peek", "preview", "reading group", "discussion",
        "about the publisher", "credits", "imprint", "colophon", "sign up", "table of",
    ];
    words < 60 || SKIP.iter().any(|s| t.contains(s))
}

pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(60).collect()
}

/// Where "Chapter {k}" (any case) starts in `s`, as a whole number.
fn find_chapter_heading(s: &str, k: usize) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    let needle = format!("chapter {k}");
    let mut from = 0;
    while let Some(i) = lower[from..].find(&needle).map(|i| i + from) {
        let end = i + needle.len();
        if !lower[end..].starts_with(|c: char| c.is_ascii_digit()) {
            return Some(i);
        }
        from = end;
    }
    None
}

/// Scanned books often arrive one page per file with no table of contents,
/// and with paragraph breaks lost. Rejoin the pages (healing sentences split
/// across them) and cut at "Chapter 1", "Chapter 2", … in order.
fn regroup_pages(chapters: &[crate::epub::Chapter]) -> Option<Vec<(String, String)>> {
    let pagey = chapters
        .iter()
        .filter(|c| {
            let t = c.title.trim().to_lowercase();
            t.is_empty() || t.starts_with("page") || t.starts_with("section") || t.chars().all(|ch| ch.is_ascii_digit())
        })
        .count();
    let mut sizes: Vec<usize> = chapters.iter().map(|c| count_words(&c.text)).collect();
    sizes.sort_unstable();
    let page_sized = sizes[sizes.len() / 2] < 600;
    if chapters.len() < 100 || (pagey * 10 < chapters.len() * 8 && !page_sized) {
        return None;
    }
    let all = chapters.iter().map(|c| c.text.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" ");
    let mut cuts = vec![];
    let mut from = 0;
    while let Some(i) = find_chapter_heading(&all[from..], cuts.len() + 1).map(|i| i + from) {
        let k = cuts.len() + 1;
        cuts.push((k, i, i + format!("chapter {k}").len()));
        from = i + 1;
    }
    if cuts.len() < 3 {
        return None;
    }
    Some(
        cuts.iter()
            .enumerate()
            .map(|(n, &(k, _, body))| {
                let end = cuts.get(n + 1).map(|c| c.1).unwrap_or(all.len());
                (format!("Chapter {k}"), all[body..end].trim().to_string())
            })
            .collect(),
    )
}

/// The story's chapters as (title, text), front matter dropped and scanned
/// pages regrouped. Also the book's own title, and whether paragraphs were lost.
pub(crate) fn story_chapters(path: &str) -> Result<(String, Vec<(String, String)>, bool), String> {
    let (found, chapters) = crate::epub::chapters(path)?;
    if let Some(regrouped) = regroup_pages(&chapters) {
        return Ok((found, regrouped, true));
    }
    let story = chapters.into_iter().filter(|c| !is_front_matter(&c.title, count_words(&c.text))).map(|c| (c.title, c.text)).collect();
    Ok((found, story, false))
}

pub fn profile_epub(path: &str, title: Option<&str>, author: Option<&str>) -> Result<Profile, String> {
    let (found, chapters, no_paragraphs) = story_chapters(path)?;
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let title = title.map(str::to_string).unwrap_or(if found.is_empty() || found == "Untitled" { stem } else { found });
    let author = author.map(str::to_string).or_else(|| crate::epub::author(path)).unwrap_or_default();
    let mut p = Profile::new(&slug(&title), &title, &author, path);
    p.no_paragraphs = no_paragraphs;
    for (t, text) in chapters {
        p.add_chapter(&t, &text);
    }
    if p.chapters.is_empty() {
        return Err(format!("No chapters with text in {path}"));
    }
    Ok(p)
}

/// The manuscript as it stands: outline order, live takes.
pub fn profile_manuscript(vault: &Path, ms: &str) -> Result<Profile, String> {
    let m = manuscript::load(vault, ms)?;
    let mut p = Profile::new(&format!("manuscript:{}", m.id), &m.title, "", "manuscript");
    for (title, text) in manuscript::cut_chapters(&m) {
        if count_words(&text) > 0 {
            p.add_chapter(title.as_deref().unwrap_or("Front matter"), &text);
        }
    }
    Ok(p)
}

// ---------------------------------------------------------------- the library

pub(crate) fn dir(vault: &Path) -> PathBuf {
    vault.join(".writepad").join("library")
}

pub fn list(vault: &Path) -> Vec<Profile> {
    let mut out: Vec<Profile> = fs::read_dir(dir(vault))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.file_name() != "target.json")
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a: &Profile, b| a.author.cmp(&b.author).then(a.title.cmp(&b.title)));
    out
}

pub fn get(vault: &Path, id: &str) -> Option<Profile> {
    serde_json::from_str(&fs::read_to_string(dir(vault).join(format!("{}.json", slug(id)))).ok()?).ok()
}

/// Measure an EPUB into the library (replacing an earlier measurement of it).
/// `title` and `author` override what the file says (scans often say nothing).
pub fn add(vault: &Path, path: &str, title: Option<&str>, author: Option<&str>) -> Result<Profile, String> {
    let mut p = profile_epub(path, title, author)?;
    p.source = store_source(vault, path);
    fs::create_dir_all(dir(vault)).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?;
    write_atomic(&dir(vault).join(format!("{}.json", p.id)), json.as_bytes())?;
    Ok(p)
}

/// How to remember a source: vault-relative (forward slashes) when the file is
/// inside the vault, so a shared or moved vault still finds it; else as given.
pub fn store_source(vault: &Path, source: &str) -> String {
    let (Ok(file), Ok(root)) = (Path::new(source).canonicalize(), vault.canonicalize()) else { return source.into() };
    match file.strip_prefix(&root) {
        Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        Err(_) => source.into(),
    }
}

/// A stored source as something to open: URLs and absolute paths as they are,
/// vault-relative paths joined onto the vault.
pub fn resolve_source(vault: &Path, source: &str) -> String {
    if source.starts_with("http://") || source.starts_with("https://") || Path::new(source).is_absolute() {
        return source.into();
    }
    vault.join(source).to_string_lossy().into_owned()
}

pub fn remove(vault: &Path, id: &str) -> Result<(), String> {
    let path = dir(vault).join(format!("{}.json", slug(id)));
    fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut w = weights(vault);
    if w.remove(id).is_some() {
        set_weights(vault, &w)?;
    }
    Ok(())
}

/// How much each book counts toward the target (book id → weight).
pub fn weights(vault: &Path) -> BTreeMap<String, f64> {
    fs::read_to_string(dir(vault).join("target.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn set_weights(vault: &Path, w: &BTreeMap<String, f64>) -> Result<(), String> {
    fs::create_dir_all(dir(vault)).map_err(|e| e.to_string())?;
    let w: BTreeMap<_, _> = w.iter().filter(|(_, v)| **v > 0.0).map(|(k, v)| (k.clone(), *v)).collect();
    write_atomic(&dir(vault).join("target.json"), serde_json::to_string_pretty(&w).unwrap().as_bytes())
}

// ---------------------------------------------------------------- statistics

/// (value, weight) samples, so books of different sizes blend by the weight
/// you give them rather than by how long they are.
pub(crate) type Dist = Vec<(f64, f64)>;

#[derive(Serialize, Clone, Default)]
pub struct Quant {
    pub p10: f64,
    pub p25: f64,
    pub median: f64,
    pub p75: f64,
    pub p90: f64,
    pub p99: f64,
    pub mean: f64,
}

fn quantile(sorted: &Dist, total: f64, q: f64) -> f64 {
    let mut acc = 0.0;
    for &(v, w) in sorted {
        acc += w;
        if acc >= q * total - 1e-9 {
            return v;
        }
    }
    sorted.last().map(|x| x.0).unwrap_or(0.0)
}

pub(crate) fn quant(d: &Dist) -> Quant {
    let mut s = d.clone();
    s.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = s.iter().map(|x| x.1).sum();
    if total <= 0.0 {
        return Quant::default();
    }
    let q = |p| quantile(&s, total, p);
    Quant { p10: q(0.10), p25: q(0.25), median: q(0.5), p75: q(0.75), p90: q(0.90), p99: q(0.99), mean: s.iter().map(|x| x.0 * x.1).sum::<f64>() / total }
}

/// Share of the distribution at or below `v`.
fn percentile(d: &Dist, v: f64) -> f64 {
    let total: f64 = d.iter().map(|x| x.1).sum();
    if total <= 0.0 {
        return 0.0;
    }
    d.iter().filter(|x| x.0 <= v).map(|x| x.1).sum::<f64>() / total
}

pub(crate) fn hist_dist(h: &[u32], scale: f64) -> Dist {
    let total: u32 = h.iter().sum();
    if total == 0 {
        return vec![];
    }
    h.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, c)| (i as f64, scale * *c as f64 / total as f64)).collect()
}

#[derive(Serialize, Clone, Default)]
pub struct Endings {
    pub dialogue: f64,
    pub question: f64,
    pub short: f64,
    pub other: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub author: String,
    pub words: u64,
    pub chapters: usize,
    pub chapter_words: Quant,
    pub sentence_words: Quant,
    pub paragraph_words: Quant,
    /// False when no source had paragraph breaks (scans).
    pub paragraphs_known: bool,
    /// How much sentence lengths vary (standard deviation / mean).
    pub sentence_spread: f64,
    pub dialogue: f64,
    pub one_sentence_paras: f64,
    pub adverbs_per_1k: f64,
    pub flesch: f64,
    pub repeated_openers_per_1k: f64,
    pub endings: Endings,
    /// Share of sentences at each length 1..=60 words (index 0 = 1 word; the last is 60+).
    pub sentence_curve: Vec<f64>,
    /// Each chapter's word count, in order.
    pub chapter_list: Vec<u32>,
}

/// One book, or several blended by weight.
pub struct Model {
    pub summary: Summary,
    sentence: Dist,
    paragraph: Dist,
}

pub fn model(parts: &[(&Profile, f64)], id: &str, title: &str, author: &str) -> Model {
    let total: f64 = parts.iter().map(|p| p.1).sum::<f64>().max(1e-9);
    let mut sentence = vec![];
    let mut paragraph = vec![];
    let mut chapter = vec![];
    let mut s = Summary { id: id.into(), title: title.into(), author: author.into(), ..Default::default() };
    let mut endings = [0.0; 4];
    // Paragraph numbers come only from books that kept their paragraphs.
    let para_total: f64 = parts.iter().filter(|p| !p.0.no_paragraphs).map(|p| p.1).sum();
    for (p, w) in parts {
        if !p.no_paragraphs && para_total > 0.0 {
            let pw = w / para_total;
            paragraph.extend(hist_dist(&p.paragraph_hist, pw));
            s.one_sentence_paras += pw * if p.paragraphs > 0 { p.one_sentence_paras as f64 / p.paragraphs as f64 } else { 0.0 };
        }
        let w = w / total;
        sentence.extend(hist_dist(&p.sentence_hist, w));
        let n = p.chapters.len().max(1) as f64;
        chapter.extend(p.chapters.iter().map(|c| (c.words as f64, w / n)));
        let ratio = |a: u64, b: u64| if b > 0 { a as f64 / b as f64 } else { 0.0 };
        s.words += p.words;
        s.chapters += p.chapters.len();
        s.dialogue += w * ratio(p.dialogue_words, p.words);
        s.adverbs_per_1k += w * 1000.0 * ratio(p.adverbs, p.words);
        s.repeated_openers_per_1k += w * 1000.0 * ratio(p.repeated_openers, p.sentences);
        s.flesch += w * (206.835 - 1.015 * ratio(p.words, p.sentences) - 84.6 * ratio(p.syllables, p.words));
        for c in &p.chapters {
            let i = match c.ending.as_str() {
                "dialogue" => 0,
                "question" => 1,
                "short" => 2,
                _ => 3,
            };
            endings[i] += w / n;
        }
    }
    s.chapter_words = quant(&chapter);
    s.sentence_words = quant(&sentence);
    s.paragraph_words = quant(&paragraph);
    s.paragraphs_known = para_total > 0.0;
    let mean = s.sentence_words.mean;
    let var = sentence.iter().map(|(v, w)| w * (v - mean).powi(2)).sum::<f64>() / sentence.iter().map(|x| x.1).sum::<f64>().max(1e-9);
    s.sentence_spread = if mean > 0.0 { var.sqrt() / mean } else { 0.0 };
    s.endings = Endings { dialogue: endings[0], question: endings[1], short: endings[2], other: endings[3] };
    let mut curve = vec![0.0; 60];
    let sw: f64 = sentence.iter().map(|x| x.1).sum::<f64>().max(1e-9);
    for (v, w) in &sentence {
        let i = (*v as usize).clamp(1, 60) - 1;
        curve[i] += w / sw;
    }
    s.sentence_curve = curve;
    if let [(p, _)] = parts {
        s.chapter_list = p.chapters.iter().map(|c| c.words).collect();
    }
    Model { summary: s, sentence, paragraph }
}

pub fn book_model(p: &Profile) -> Model {
    model(&[(p, 1.0)], &p.id, &p.title, &p.author)
}

/// The blend of the weighted books, if any have weight.
pub fn target(vault: &Path) -> Option<Model> {
    let w = weights(vault);
    let books = list(vault);
    let parts: Vec<(&Profile, f64)> = books.iter().filter_map(|b| w.get(&b.id).filter(|v| **v > 0.0).map(|v| (b, *v))).collect();
    if parts.is_empty() {
        return None;
    }
    let name = parts
        .iter()
        .map(|(b, v)| {
            let who = if b.author.is_empty() { &b.title } else { &b.author };
            format!("{:.0}% {}", 100.0 * v / parts.iter().map(|p| p.1).sum::<f64>(), who)
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let mut m = model(&parts, "target", "Target", &name);
    m.summary.chapter_list.clear(); // the band stands for the target, not its books' chapters
    Some(m)
}

#[derive(Serialize)]
pub struct Report {
    pub books: Vec<Summary>,
    pub weights: BTreeMap<String, f64>,
    pub target: Option<Summary>,
    pub manuscript: Option<Summary>,
}

pub fn report(vault: &Path, ms: Option<&str>) -> Report {
    Report {
        books: list(vault).iter().map(|b| book_model(b).summary).collect(),
        weights: weights(vault),
        target: target(vault).map(|m| m.summary),
        manuscript: ms.and_then(|id| profile_manuscript(vault, id).ok()).map(|p| book_model(&p).summary),
    }
}

// ---------------------------------------------------------------- the linter

#[derive(Serialize, Clone)]
pub struct Flag {
    /// UTF-16 offsets, as the editor counts.
    pub from: usize,
    pub to: usize,
    /// run | long | dense | opener
    pub kind: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct Cadence {
    pub words: usize,
    pub sentences: usize,
    pub sentence_median: f64,
    pub dialogue: f64,
    /// The target's middle half of chapter lengths, and its median.
    pub chapter_band: Option<(f64, f64, f64)>,
    pub sentence_target: Option<f64>,
    pub dialogue_target: Option<f64>,
    pub target_name: Option<String>,
    pub flags: Vec<Flag>,
}

const PRONOUNS: &[&str] = &["i", "you", "he", "she", "we", "they", "it"];

fn utf16(text: &str, byte: usize) -> usize {
    text[..byte].encode_utf16().count()
}

/// Measure a scene and flag stretches that drift from the target. One long
/// sentence is a choice; four in a row is a pattern worth a look.
pub fn cadence(text: &str, target: Option<&Model>) -> Cadence {
    let paras = measure(text);
    let sents: Vec<&Sentence> = paras.iter().flat_map(|p| &p.sentences).collect();
    let words: usize = paras.iter().map(|p| p.words).sum();
    let mut lens: Vec<usize> = sents.iter().map(|s| s.words).collect();
    lens.sort_unstable();
    let mut c = Cadence {
        words,
        sentences: sents.len(),
        sentence_median: lens.get(lens.len() / 2).copied().unwrap_or(0) as f64,
        dialogue: if words > 0 { paras.iter().map(|p| p.dialogue_words).sum::<usize>() as f64 / words as f64 } else { 0.0 },
        chapter_band: None,
        sentence_target: None,
        dialogue_target: None,
        target_name: None,
        flags: vec![],
    };
    let Some(t) = target else { return c };
    let ts = &t.summary;
    c.chapter_band = Some((ts.chapter_words.p25, ts.chapter_words.median, ts.chapter_words.p75));
    c.sentence_target = Some(ts.sentence_words.median);
    c.dialogue_target = Some(ts.dialogue);
    c.target_name = Some(ts.author.clone());
    let flag = |from: usize, to: usize, kind: &str, message: String| Flag { from: utf16(text, from), to: utf16(text, to), kind: kind.into(), message };

    // Runs of long sentences.
    let long = ts.sentence_words.p75.max(ts.sentence_words.median + 3.0);
    let mut in_run = vec![false; sents.len()];
    let mut i = 0;
    while i < sents.len() {
        let mut j = i;
        while j < sents.len() && sents[j].words as f64 > long {
            j += 1;
        }
        if j - i >= 4 {
            in_run[i..j].iter_mut().for_each(|x| *x = true);
            c.flags.push(flag(
                sents[i].start,
                sents[j - 1].end,
                "run",
                format!("{} long sentences in a row, each over {long:.0} words. Your target's typical sentence is {:.0}.", j - i, ts.sentence_words.median),
            ));
        }
        i = j.max(i + 1);
    }
    // Single sentences longer than nearly all of the target's.
    for (k, s) in sents.iter().enumerate() {
        if !in_run[k] && s.words as f64 > ts.sentence_words.p99.max(30.0) {
            let pct = 100.0 * percentile(&t.sentence, s.words as f64 - 1.0);
            c.flags.push(flag(s.start, s.end, "long", format!("{} words. Longer than {pct:.0}% of sentences in your target.", s.words)));
        }
    }
    // Dense paragraphs (only if the target knows what a paragraph looks like).
    let dense = (ts.paragraph_words.p90 * 1.5).max(ts.paragraph_words.p99);
    for p in paras.iter().filter(|_| ts.paragraphs_known) {
        if p.words as f64 > dense {
            let pct = 100.0 * percentile(&t.paragraph, p.words as f64 - 1.0);
            c.flags.push(flag(p.start, p.end, "dense", format!("{}-word paragraph. Longer than {pct:.0}% of your target's paragraphs.", p.words)));
        }
    }
    // The same opening word three times running; four for a pronoun, since
    // first- and second-person prose leans on "I" and "You" by nature.
    let mut k = 0;
    while k < sents.len() {
        let w = &sents[k].opener;
        let mut end = k;
        while end + 1 < sents.len() && sents[end + 1].opener == *w {
            end += 1;
        }
        let n = end - k + 1;
        let need = if PRONOUNS.contains(&w.as_str()) { 4 } else { 3 };
        if !w.is_empty() && n >= need {
            for s in &sents[k..=end] {
                c.flags.push(flag(s.opener_span.0, s.opener_span.1, "opener", format!("{n} sentences in a row open with “{}”.", &text[s.opener_span.0..s.opener_span.1])));
            }
        }
        k = end + 1;
    }
    c.flags.sort_by_key(|f| f.from);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_inside_the_vault_are_stored_relative() {
        let vault = std::env::temp_dir().join(format!("wp-src-{}", std::process::id()));
        fs::create_dir_all(vault.join("library")).unwrap();
        let book = vault.join("library").join("A Book.epub");
        fs::write(&book, b"x").unwrap();
        let stored = store_source(&vault, &book.to_string_lossy());
        assert_eq!(stored, "library/A Book.epub");
        assert_eq!(Path::new(&resolve_source(&vault, &stored)).canonicalize().unwrap(), book.canonicalize().unwrap());
        // Outside the vault, or a URL: kept as given.
        let outside = std::env::temp_dir().join("elsewhere.epub").to_string_lossy().into_owned();
        assert_eq!(store_source(&vault, &outside), outside);
        assert_eq!(resolve_source(&vault, "https://imsdb.com/x.html"), "https://imsdb.com/x.html");
        assert_eq!(resolve_source(&vault, &outside), outside);
        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn splits_sentences_like_a_reader() {
        let text = "“Stop!” he said. Mr. Lee waited. She ran… Then nothing. J. R. R. Tolkien wrote it? Yes.";
        let s = sentences(text, 0);
        let got: Vec<&str> = s.iter().map(|x| &text[x.start..x.end]).collect();
        assert_eq!(got, ["“Stop!” he said.", "Mr. Lee waited.", "She ran…", "Then nothing.", "J. R. R. Tolkien wrote it?", "Yes."]);
        assert_eq!(s[0].opener, "stop");
        assert_eq!(count_words("Don't stop—well-known 2041 rules."), 5);
        // A scan that dropped a closing quote: the cap stops it eating the narration.
        let lost = format!("“Hi there, he said. {}", "Narration goes on. ".repeat(40));
        assert_eq!(dialogue_words(&lost, Some(60)), 60);
        assert_eq!(dialogue_words("“Hi,” she said. “Bye.”", Some(60)), 2);
    }

    #[test]
    fn measures_paragraphs_dialogue_and_endings() {
        let text = "---\ntype: x\n---\n# Title\n\n“Hello there,” she said. He nodded.\n\n* * *\n\nWhat now?\n\n*If you run, turn to 12.*\n";
        let paras = measure(text);
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].dialogue_words, 2);
        assert_eq!(paras[0].sentences.len(), 2);
        assert_eq!(ending_of(&paras, text), "question");
        let mut p = Profile::new("x", "X", "", "");
        p.add_chapter("One", text);
        assert_eq!(p.words, 8);
        assert_eq!(p.one_sentence_paras, 1);
    }

    #[test]
    fn blends_by_weight_and_flags_runs() {
        let mut short = Profile::new("a", "A", "Short", "");
        let mut long = Profile::new("b", "B", "Long", "");
        short.add_chapter("1", &"Tiny one here. ".repeat(200));
        long.add_chapter("1", &"This one is a far longer sentence with many more words in it. ".repeat(200));
        let m = model(&[(&short, 3.0), (&long, 1.0)], "t", "T", "");
        assert_eq!(m.summary.sentence_words.median, 3.0);
        assert_eq!(m.summary.sentence_words.p90, 13.0);

        let scene = format!("{}\n\nI ran. I hid. I waited.\n", "This one is a far longer sentence with many more words in it. ".repeat(5));
        let c = cadence(&scene, Some(&m));
        let kinds: Vec<&str> = c.flags.iter().map(|f| f.kind.as_str()).collect();
        // The five long ones all open with "This"; three "I"s in a row is just first person.
        assert_eq!(kinds.iter().filter(|k| **k == "opener").count(), 5);
        assert_eq!(kinds[0], "run");
        assert!(c.flags[0].message.starts_with("5 long sentences"));
    }

    #[test]
    fn offsets_are_utf16() {
        let m = model(&[(&{
            let mut p = Profile::new("a", "A", "", "");
            p.add_chapter("1", &"One two three. ".repeat(50));
            p
        }, 1.0)], "t", "T", "");
        let text = "“É” xo. Yo said this. Yo said that. Yo said more.";
        let c = cadence(text, Some(&m));
        let f = &c.flags[0];
        let u: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&u[f.from..f.to]).unwrap(), "Yo");
    }

    /// WP_VAULT=… WP_MS=… cargo test dump_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dump_report() {
        let v = std::env::var("WP_VAULT").unwrap();
        let ms = std::env::var("WP_MS").ok();
        println!("REPORT{}", serde_json::to_string(&report(Path::new(&v), ms.as_deref())).unwrap());
    }

    #[test]
    #[ignore]
    fn dump_rpo() {
        let p = profile_epub(&std::env::var("WP_EPUB").unwrap(), None, None).unwrap();
        for c in &p.chapters {
            println!("{:>6} {:<9} {}", c.words, c.ending, c.title);
        }
        println!("{}", serde_json::to_string_pretty(&book_model(&p).summary).unwrap());
    }
}
