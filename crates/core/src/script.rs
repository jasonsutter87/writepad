//! Screenplays. A script is read into its elements (scene headings, action,
//! character cues, parentheticals, dialogue), measured like a book is, and
//! lined up against the novel it adapts. As with books, the library keeps
//! numbers only; the script's text is read from its source when needed.

use crate::profile::{self, quant, Dist, Quant};
use crate::manuscript::write_atomic;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- reading

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Heading,
    Action,
    Cue,
    Paren,
    Dialogue,
}

#[derive(Clone, Debug, Serialize)]
pub struct Element {
    pub kind: Kind,
    pub text: String,
}

/// The script's text from a URL (IMSDb-style HTML with a `<pre>`), a .pdf, or
/// a plain .txt / .fountain file.
pub fn load_source(source: &str) -> Result<String, String> {
    let raw = if source.starts_with("http://") || source.starts_with("https://") {
        let mut resp = ureq::get(source).header("User-Agent", "Mozilla/5.0 (WRITEpad)").call().map_err(|e| format!("{source}: {e}"))?;
        let bytes = resp.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        decode(&bytes)
    } else if source.to_lowercase().ends_with(".pdf") {
        let bytes = fs::read(source).map_err(|e| format!("{source}: {e}"))?;
        pdf_extract::extract_text_from_mem(&bytes).map_err(|e| format!("{source}: {e}"))?
    } else {
        decode(&fs::read(source).map_err(|e| format!("{source}: {e}"))?)
    };
    Ok(if raw.contains("<pre") || raw.contains("<html") || raw.contains("<HTML") { html_text(&raw) } else { raw })
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect())
}

/// The `<pre>` block if there is one, tags stripped and entities decoded.
fn html_text(html: &str) -> String {
    let lower = html.to_lowercase();
    let body = match (lower.find("<pre"), lower.rfind("</pre>")) {
        (Some(a), Some(b)) if b > a => &html[a..b],
        _ => html,
    };
    let mut out = String::with_capacity(body.len());
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn is_caps(t: &str) -> bool {
    t.chars().any(|c| c.is_alphabetic()) && !t.chars().any(|c| c.is_lowercase())
}

fn is_heading(t: &str) -> bool {
    if t.starts_with('.') && t.len() > 1 && !t.starts_with("..") {
        return true; // Fountain's forced heading
    }
    let u = t.to_uppercase();
    is_caps(t) && ["INT.", "EXT.", "INT ", "EXT ", "INT/", "EXT/", "I/E", "E/I", "EST.", "INTERIOR", "EXTERIOR"].iter().any(|p| u.starts_with(p))
}

/// Page furniture that isn't part of the story.
fn is_furniture(t: &str) -> bool {
    let u = t.trim_matches(|c: char| c == '(' || c == ')').trim();
    let page_no = t.trim_end_matches('.').chars().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()) && t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.len() <= 6;
    page_no
        || u.starts_with("CONTINUED")
        || u == "MORE"
        || (is_caps(t) && (t.ends_with("TO:") || t.ends_with("IN:") || t.starts_with("FADE") || t.starts_with("CUT TO") || t == "THE END" || t.starts_with("DISSOLVE")))
}

/// A character's name, without (V.O.), (CONT'D), (OVER RADIO) and the like.
pub fn character_name(cue: &str) -> String {
    let base = cue.split('(').next().unwrap_or(cue).trim().trim_start_matches('@');
    base.trim_end_matches(|c: char| c == '^' || c == ':').trim().to_string()
}

/// Split a script into elements. Works from capitals and blank lines, not
/// indentation, so it reads IMSDb pages, PDFs and Fountain alike.
pub fn parse(text: &str) -> (Vec<Element>, String, String) {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    let mut els: Vec<Element> = vec![];
    let mut in_speech = false;
    let mut started = false;
    let mut title_page: Vec<String> = vec![];
    let next_text = |i: usize| lines[i + 1..].iter().map(|l| l.trim()).find(|t| !t.is_empty() && !is_furniture(t));
    let next_is_text = |i: usize| lines.get(i + 1).is_some_and(|l| !l.trim().is_empty()) || {
        // A page break can fall between a cue and its first line.
        lines.get(i + 1).is_some_and(|l| is_furniture(l.trim())) && next_text(i).is_some()
    };
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() {
            in_speech = false;
            continue;
        }
        if is_heading(t) {
            started = true;
            in_speech = false;
            els.push(Element { kind: Kind::Heading, text: t.trim_start_matches('.').to_string() });
            continue;
        }
        if !started {
            title_page.push(t.to_string());
            continue;
        }
        if is_furniture(t) {
            continue;
        }
        if in_speech {
            let kind = if t.starts_with('(') { Kind::Paren } else { Kind::Dialogue };
            match els.last_mut() {
                Some(e) if e.kind == kind && kind == Kind::Dialogue => {
                    e.text.push(' ');
                    e.text.push_str(t);
                }
                _ => els.push(Element { kind, text: t.to_string() }),
            }
            continue;
        }
        let name = character_name(t);
        let cue = (is_caps(t) || t.starts_with('@'))
            && !name.is_empty()
            && name.split_whitespace().count() <= 4
            && t.len() <= 50
            && !name.ends_with(['.', '!', '?', ','])
            && next_is_text(i);
        if cue {
            in_speech = true;
            els.push(Element { kind: Kind::Cue, text: t.to_string() });
            continue;
        }
        match els.last_mut() {
            Some(e) if e.kind == Kind::Action && i > 0 && !lines[i - 1].trim().is_empty() => {
                e.text.push(' ');
                e.text.push_str(t);
            }
            _ => els.push(Element { kind: Kind::Action, text: t.to_string() }),
        }
    }
    let title = title_page.first().cloned().unwrap_or_default();
    let writer = title_page
        .iter()
        .position(|l| {
            let l = l.to_lowercase();
            l.contains("written by") || l.contains("screenplay by")
        })
        .and_then(|i| {
            let l = &title_page[i];
            let after = l.to_lowercase().find(" by").map(|p| l[p + 3..].trim().to_string()).filter(|s| !s.is_empty());
            after.or_else(|| title_page.get(i + 1).cloned())
        })
        .unwrap_or_default();
    (els, title, writer)
}

// ---------------------------------------------------------------- measuring

/// Rendered lines in standard screenplay layout: action wraps at about 60
/// characters, dialogue at 35, parentheticals at 25. A page is about 55 lines.
const LINES_PER_PAGE: f64 = 55.0;
fn wrapped(text: &str, width: usize) -> usize {
    let mut lines = 1;
    let mut col = 0;
    for w in text.split_whitespace() {
        let n = w.chars().count();
        if col > 0 && col + 1 + n > width {
            lines += 1;
            col = n;
        } else {
            col += if col > 0 { 1 + n } else { n };
        }
    }
    lines
}

fn layout_lines(e: &Element) -> usize {
    match e.kind {
        Kind::Heading => 2,
        Kind::Action => wrapped(&e.text, 60) + 1,
        Kind::Cue => 1,
        Kind::Paren => wrapped(&e.text, 25),
        Kind::Dialogue => wrapped(&e.text, 35) + 1,
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SceneStat {
    pub heading: String,
    /// INT, EXT or INT/EXT
    pub place: String,
    pub location: String,
    pub time: String,
    pub pages: f64,
    pub action_words: u32,
    pub dialogue_words: u32,
    pub speeches: u32,
    pub characters: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CharStat {
    pub name: String,
    pub words: u32,
    pub speeches: u32,
    pub first_scene: u32,
}

/// Everything measured about one script. No text is kept.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Script {
    pub id: String,
    pub title: String,
    pub writer: String,
    pub source: String,
    pub measured: u64,
    pub pages: f64,
    pub scenes: Vec<SceneStat>,
    /// Words per speech (index = words, capped).
    pub speech_hist: Vec<u32>,
    /// Rendered lines per action paragraph (capped).
    pub action_hist: Vec<u32>,
    pub characters: Vec<CharStat>,
}

fn words(t: &str) -> u32 {
    profile::count_words(t) as u32
}

fn split_heading(h: &str) -> (String, String, String) {
    let u = h.to_uppercase();
    let place = if u.starts_with("INT/EXT") || u.starts_with("INT./EXT") || u.starts_with("I/E") || u.starts_with("E/I") || u.starts_with("EXT/INT") {
        "INT/EXT"
    } else if u.starts_with("EXT") {
        "EXT"
    } else if u.starts_with("INT") {
        "INT"
    } else {
        ""
    };
    let rest = h.split_once(['.', ' ']).map(|x| x.1).unwrap_or("").trim_start_matches(['.', '/', ' ']).trim();
    let rest = rest.trim_start_matches("EXT.").trim_start_matches("INT.").trim();
    let (loc, time) = match rest.rsplit_once(" - ") {
        Some((l, t)) if t.split_whitespace().count() <= 3 => (l.trim(), t.trim()),
        _ => (rest, ""),
    };
    // "MARS - HAB - DAY": the first part is where in the world, the rest is the room.
    (place.into(), loc.to_string(), time.to_string())
}

pub fn measure(id: &str, title: &str, writer: &str, source: &str, els: &[Element]) -> Script {
    let mut s = Script {
        id: id.into(),
        title: title.into(),
        writer: writer.into(),
        source: source.into(),
        measured: crate::manuscript::now(),
        speech_hist: vec![0; 201],
        action_hist: vec![0; 31],
        ..Default::default()
    };
    let mut chars: HashMap<String, CharStat> = HashMap::new();
    let mut total_lines = 0usize;
    let mut speaker: Option<String> = None;
    let mut speech_words = 0u32;
    let close_speech = |s: &mut Script, w: &mut u32| {
        if *w > 0 {
            s.speech_hist[(*w as usize).min(200)] += 1;
        }
        *w = 0;
    };
    for e in els {
        let lines = layout_lines(e);
        total_lines += lines;
        if e.kind != Kind::Dialogue && e.kind != Kind::Paren {
            close_speech(&mut s, &mut speech_words);
        }
        match e.kind {
            Kind::Heading => {
                let (place, location, time) = split_heading(&e.text);
                s.scenes.push(SceneStat { heading: e.text.clone(), place, location, time, ..Default::default() });
            }
            _ if s.scenes.is_empty() => {}
            Kind::Action => {
                s.action_hist[(lines - 1).min(30)] += 1;
                s.scenes.last_mut().unwrap().action_words += words(&e.text);
            }
            Kind::Cue => {
                let name = character_name(&e.text);
                let n = s.scenes.len() as u32;
                let sc = s.scenes.last_mut().unwrap();
                sc.speeches += 1;
                if !sc.characters.contains(&name) {
                    sc.characters.push(name.clone());
                }
                let c = chars.entry(name.clone()).or_insert_with(|| CharStat { name: name.clone(), first_scene: n, ..Default::default() });
                c.speeches += 1;
                speaker = Some(name);
            }
            Kind::Paren => {}
            Kind::Dialogue => {
                let w = words(&e.text);
                speech_words += w;
                s.scenes.last_mut().unwrap().dialogue_words += w;
                if let Some(c) = speaker.as_ref().and_then(|n| chars.get_mut(n)) {
                    c.words += w;
                }
            }
        }
        if let Some(sc) = s.scenes.last_mut() {
            sc.pages += lines as f64 / LINES_PER_PAGE;
        }
    }
    close_speech(&mut s, &mut speech_words);
    s.pages = total_lines as f64 / LINES_PER_PAGE;
    let mut cs: Vec<CharStat> = chars.into_values().filter(|c| c.words > 0).collect();
    cs.sort_by(|a, b| b.words.cmp(&a.words));
    cs.truncate(40);
    s.characters = cs;
    s
}

// ---------------------------------------------------------------- summary

#[derive(Serialize, Clone, Default)]
pub struct CharShare {
    pub name: String,
    pub share: f64,
    pub speeches: u32,
    /// Page where they first speak.
    pub first_page: f64,
}

#[derive(Serialize, Clone)]
pub struct PaceScene {
    pub start: f64,
    pub pages: f64,
    pub dialogue: f64,
    pub heading: String,
}

#[derive(Serialize, Clone, Default)]
pub struct ScriptSummary {
    pub id: String,
    pub title: String,
    pub writer: String,
    pub source: String,
    pub pages: f64,
    pub scenes: usize,
    pub scene_pages: Quant,
    pub dialogue: f64,
    pub speech_words: Quant,
    pub action_lines: Quant,
    pub speeches_per_page: f64,
    pub interior: f64,
    pub night: f64,
    pub locations: usize,
    /// Share of scenes set somewhere other than the scene before.
    pub location_changes: f64,
    pub characters: Vec<CharShare>,
    pub pacing: Vec<PaceScene>,
}

pub fn summarize(s: &Script) -> ScriptSummary {
    let scene_pages: Dist = s.scenes.iter().map(|x| (x.pages, 1.0)).collect();
    let (aw, dw): (u32, u32) = s.scenes.iter().fold((0, 0), |a, x| (a.0 + x.action_words, a.1 + x.dialogue_words));
    let speeches: u32 = s.scenes.iter().map(|x| x.speeches).sum();
    let n = s.scenes.len().max(1) as f64;
    let mut starts = vec![];
    let mut at = 0.0;
    for sc in &s.scenes {
        starts.push(at);
        at += sc.pages;
    }
    let first_page = |scene: u32| starts.get(scene.saturating_sub(1) as usize).copied().unwrap_or(0.0) + 1.0;
    let mut locs: HashSet<&str> = HashSet::new();
    let mut changes = 0;
    for (i, sc) in s.scenes.iter().enumerate() {
        let root = sc.location.split(" - ").next().unwrap_or("").trim();
        locs.insert(root);
        if i > 0 && s.scenes[i - 1].location.split(" - ").next().unwrap_or("").trim() != root {
            changes += 1;
        }
    }
    ScriptSummary {
        id: s.id.clone(),
        title: s.title.clone(),
        writer: s.writer.clone(),
        source: s.source.clone(),
        pages: s.pages,
        scenes: s.scenes.len(),
        scene_pages: quant(&scene_pages),
        dialogue: if aw + dw > 0 { dw as f64 / (aw + dw) as f64 } else { 0.0 },
        speech_words: quant(&profile::hist_dist(&s.speech_hist, 1.0)),
        action_lines: quant(&profile::hist_dist(&s.action_hist, 1.0)),
        speeches_per_page: if s.pages > 0.0 { speeches as f64 / s.pages } else { 0.0 },
        interior: s.scenes.iter().filter(|x| x.place == "INT").count() as f64 / n,
        night: s.scenes.iter().filter(|x| x.time.to_uppercase().contains("NIGHT")).count() as f64 / n,
        locations: locs.len(),
        location_changes: if s.scenes.len() > 1 { changes as f64 / (s.scenes.len() - 1) as f64 } else { 0.0 },
        characters: s
            .characters
            .iter()
            .take(10)
            .map(|c| CharShare { name: c.name.clone(), share: if dw > 0 { c.words as f64 / dw as f64 } else { 0.0 }, speeches: c.speeches, first_page: first_page(c.first_scene) })
            .collect(),
        pacing: s
            .scenes
            .iter()
            .zip(&starts)
            .map(|(sc, &start)| PaceScene {
                start,
                pages: sc.pages,
                dialogue: if sc.action_words + sc.dialogue_words > 0 { sc.dialogue_words as f64 / (sc.action_words + sc.dialogue_words) as f64 } else { 0.0 },
                heading: sc.heading.clone(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------- the library

fn dir(vault: &Path) -> PathBuf {
    profile::dir(vault).join("scripts")
}

pub fn list(vault: &Path) -> Vec<Script> {
    let mut out: Vec<Script> = fs::read_dir(dir(vault))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

pub fn get(vault: &Path, id: &str) -> Result<Script, String> {
    let p = dir(vault).join(format!("{}.json", profile::slug(id)));
    serde_json::from_str(&fs::read_to_string(&p).map_err(|_| format!("No script {id:?} in the library."))?).map_err(|e| e.to_string())
}

/// Title-case a shouty title page ("THE MARTIAN" → "The Martian").
fn tidy_title(t: &str) -> String {
    if !is_caps(t) {
        return t.to_string();
    }
    t.split_whitespace()
        .enumerate()
        .map(|(i, w)| {
            let lw = w.to_lowercase();
            if i > 0 && ["a", "an", "the", "of", "and", "in", "on", "to", "for"].contains(&lw.as_str()) {
                lw
            } else {
                let mut c = lw.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read a script from a URL or file and measure it into the library.
pub fn add(vault: &Path, source: &str, title: Option<&str>, writer: Option<&str>) -> Result<Script, String> {
    let text = load_source(source)?;
    let (els, found_title, found_writer) = parse(&text);
    if els.iter().filter(|e| e.kind == Kind::Heading).count() < 3 {
        return Err(format!("{source} doesn't look like a screenplay (no INT./EXT. scene headings)."));
    }
    let title = title.map(str::to_string).unwrap_or_else(|| tidy_title(&found_title));
    let writer = writer.map(str::to_string).unwrap_or(found_writer);
    let s = measure(&profile::slug(&title), &title, &writer, &profile::store_source(vault, source), &els);
    fs::create_dir_all(dir(vault)).map_err(|e| e.to_string())?;
    write_atomic(&dir(vault).join(format!("{}.json", s.id)), serde_json::to_string_pretty(&s).unwrap().as_bytes())?;
    Ok(s)
}

pub fn remove(vault: &Path, id: &str) -> Result<(), String> {
    let p = dir(vault).join(format!("{}.json", profile::slug(id)));
    fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn report(vault: &Path) -> Vec<ScriptSummary> {
    list(vault).iter().map(summarize).collect()
}

/// The elements back as Fountain text, which WRITEpad's editor lays out as a
/// script and can read aloud with a cast.
pub fn fountain(els: &[Element]) -> String {
    let mut out = String::new();
    for (i, e) in els.iter().enumerate() {
        let prev = i.checked_sub(1).map(|k| els[k].kind);
        let in_speech = matches!(prev, Some(Kind::Cue | Kind::Paren | Kind::Dialogue)) && matches!(e.kind, Kind::Paren | Kind::Dialogue);
        if !out.is_empty() && !in_speech {
            out.push_str("\n\n");
        } else if in_speech {
            out.push('\n');
        }
        match e.kind {
            Kind::Heading => out.push_str(&e.text.to_uppercase()),
            _ => out.push_str(&e.text),
        }
    }
    out.push('\n');
    out
}

/// Open a Library script as a manuscript: its text, laid out as Fountain, in
/// blocks of about five pages that start at scene headings. This copies the
/// script's text into the vault, unlike measuring it.
pub fn open_as_manuscript(vault: &Path, id: &str) -> Result<String, String> {
    let s = get(vault, id)?;
    let (els, _, _) = parse(&load_source(&profile::resolve_source(vault, &s.source))?);
    let mut sections: Vec<(String, String)> = vec![];
    let mut chunk: Vec<Element> = vec![];
    let mut lines = 0usize;
    let mut page = 1.0f64;
    let mut start_page = 1.0f64;
    let flush = |chunk: &mut Vec<Element>, sections: &mut Vec<(String, String)>, start: f64| {
        if let Some(h) = chunk.iter().find(|e| e.kind == Kind::Heading) {
            sections.push((format!("p.{} · {}", start.floor() as u32, h.text.to_uppercase()), fountain(chunk)));
        }
        chunk.clear();
    };
    for e in els {
        if e.kind == Kind::Heading && lines as f64 >= 5.0 * LINES_PER_PAGE {
            flush(&mut chunk, &mut sections, start_page);
            lines = 0;
            start_page = page;
        }
        let n = layout_lines(&e);
        lines += n;
        page += n as f64 / LINES_PER_PAGE;
        chunk.push(e);
    }
    flush(&mut chunk, &mut sections, start_page);
    crate::manuscript::from_sections(vault, &format!("{} (script)", s.title), "script", sections)
}

// ---------------------------------------------------------------- adaptation

const STOP: &[&str] = &[
    "the", "and", "that", "with", "you", "for", "this", "but", "have", "not", "are", "was", "his", "her", "they", "she", "him", "what", "all",
    "from", "just", "can", "out", "get", "like", "there", "about", "one", "now", "then", "into", "them", "their", "your", "our", "we're", "it's",
    "don't", "i'm", "has", "had", "were", "been", "will", "would", "could", "should", "him", "who", "how", "when", "where", "why", "some",
    "more", "back", "over", "here", "than", "its", "let's", "that's", "i'll", "you're", "he's", "she's", "off", "down", "very", "only",
    "also", "any", "each", "other", "after", "before", "know", "think", "going", "right", "okay", "yeah", "well", "look", "looks", "still",
];

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'))
        .map(|w| w.replace('’', "'").to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

fn terms(text: &str) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    for w in tokens(text) {
        if w.len() >= 3 && !STOP.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()) {
            *m.entry(w).or_insert(0.0) += 1.0;
        }
    }
    m
}

/// TF-IDF vectors, L2-normalised.
fn vectors(docs: &[HashMap<String, f64>], df: &HashMap<String, f64>, n: f64) -> Vec<HashMap<String, f64>> {
    docs.iter()
        .map(|d| {
            let mut v: HashMap<String, f64> = d.iter().map(|(w, tf)| (w.clone(), (1.0 + tf.ln()) * (n / df.get(w).copied().unwrap_or(1.0)).ln().max(0.0))).collect();
            let norm = v.values().map(|x| x * x).sum::<f64>().sqrt().max(1e-9);
            v.values_mut().for_each(|x| *x /= norm);
            v
        })
        .collect()
}

fn cosine(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let (small, big) = if a.len() < b.len() { (a, b) } else { (b, a) };
    small.iter().map(|(w, x)| x * big.get(w).copied().unwrap_or(0.0)).sum()
}

#[derive(Serialize)]
pub struct AdaptChapter {
    pub title: String,
    pub words: usize,
    pub scenes: Vec<usize>,
    pub pages: f64,
}

#[derive(Serialize)]
pub struct AdaptScene {
    pub heading: String,
    pub pages: f64,
    /// The chapter this scene draws on most, if any clearly.
    pub chapter: Option<usize>,
    pub score: f64,
    /// Placed by its neighbours (a short bridge between two scenes from one chapter).
    pub bridged: bool,
    /// Share of this scene's dialogue lifted word for word from the book.
    pub verbatim: f64,
}

#[derive(Serialize)]
pub struct Kept {
    pub speaker: String,
    pub text: String,
    pub chapter: Option<usize>,
}

#[derive(Serialize)]
pub struct Adaptation {
    pub book: String,
    pub script: String,
    pub book_words: usize,
    pub script_pages: f64,
    pub chapters: Vec<AdaptChapter>,
    pub scenes: Vec<AdaptScene>,
    /// Share of the script's dialogue words found word for word in the book.
    pub verbatim: f64,
    pub kept: Vec<Kept>,
    /// Scenes (and their pages) with no clear source in the book.
    pub invented_scenes: usize,
    pub invented_pages: f64,
    pub cut_chapters: usize,
}

struct SceneText {
    heading: String,
    text: String,
    speeches: Vec<(String, String)>,
    lines: usize,
}

fn scene_texts(els: &[Element]) -> Vec<SceneText> {
    let mut out: Vec<SceneText> = vec![];
    let mut speaker = String::new();
    for e in els {
        if e.kind == Kind::Heading {
            out.push(SceneText { heading: e.text.clone(), text: String::new(), speeches: vec![], lines: 0 });
        }
        let Some(sc) = out.last_mut() else { continue };
        sc.lines += layout_lines(e);
        match e.kind {
            Kind::Cue => speaker = character_name(&e.text),
            Kind::Dialogue => sc.speeches.push((speaker.clone(), e.text.clone())),
            _ => {}
        }
        if matches!(e.kind, Kind::Action | Kind::Dialogue) {
            sc.text.push(' ');
            sc.text.push_str(&e.text);
        }
    }
    out
}

/// Line a book up against its film script: which chapters each scene draws
/// on, what was cut, what was invented, and which lines survived verbatim.
/// Reads both sources fresh; nothing is stored.
pub fn adapt(vault: &Path, book_id: &str, script_id: &str) -> Result<Adaptation, String> {
    let book = profile::get(vault, book_id).ok_or(format!("No book {book_id:?} in the library."))?;
    let script = get(vault, script_id)?;
    let (_, chapters, _) = profile::story_chapters(&profile::resolve_source(vault, &book.source)).map_err(|e| format!("Couldn't reread the book ({e}). Was the EPUB moved?"))?;
    let (els, _, _) = parse(&load_source(&profile::resolve_source(vault, &script.source))?);
    let scenes = scene_texts(&els);

    // Which chapter is each scene about? TF-IDF over chapters and scenes, then
    // a pass that prefers following the book's order but lets the film jump.
    let ch_terms: Vec<_> = chapters.iter().map(|c| terms(&c.1)).collect();
    let sc_terms: Vec<_> = scenes.iter().map(|s| terms(&format!("{} {}", s.heading, s.text))).collect();
    let mut df: HashMap<String, f64> = HashMap::new();
    for d in ch_terms.iter().chain(&sc_terms) {
        for w in d.keys() {
            *df.entry(w.clone()).or_insert(0.0) += 1.0;
        }
    }
    let n = (ch_terms.len() + sc_terms.len()) as f64;
    let cv = vectors(&ch_terms, &df, n);
    let sv = vectors(&sc_terms, &df, n);
    let sim: Vec<Vec<f64>> = sv.iter().map(|s| cv.iter().map(|c| cosine(s, c)).collect()).collect();
    let nc = chapters.len();
    // Viterbi: reward similarity, charge a little for jumping backwards or far ahead.
    let jump = |from: usize, to: usize| -> f64 {
        if to >= from && to <= from + 2 {
            0.0
        } else if to > from {
            0.02
        } else {
            0.04
        }
    };
    let mut best: Vec<Vec<usize>> = vec![];
    let mut score: Vec<f64> = sim.first().cloned().unwrap_or_default();
    for row in sim.iter().skip(1) {
        let mut next = vec![f64::MIN; nc];
        let mut back = vec![0; nc];
        // The best predecessor for each chapter, scanning all is fine at book scale.
        for to in 0..nc {
            for from in 0..nc {
                let v = score[from] - jump(from, to);
                if v > next[to] {
                    next[to] = v;
                    back[to] = from;
                }
            }
            next[to] += row[to];
        }
        best.push(back);
        score = next;
    }
    let mut path = vec![0; sim.len()];
    if let Some((i, _)) = score.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) {
        if let Some(last) = path.last_mut() {
            *last = i;
        }
        for k in (1..sim.len()).rev() {
            path[k - 1] = best[k - 1][path[k]];
        }
    }

    // Verbatim dialogue: 6-word runs of the script found in the book.
    const K: usize = 6;
    let mut grams: HashMap<u64, usize> = HashMap::new();
    let hash = |ws: &[String]| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ws.hash(&mut h);
        h.finish()
    };
    for (ci, (_, text)) in chapters.iter().enumerate() {
        let t = tokens(text);
        for w in t.windows(K) {
            grams.entry(hash(w)).or_insert(ci);
        }
    }
    let mut kept = vec![];
    let mut total_dw = 0usize;
    let mut kept_dw = 0usize;
    let mut out_scenes = vec![];
    for (i, sc) in scenes.iter().enumerate() {
        let mut dw = 0;
        let mut kw = 0;
        for (who, line) in &sc.speeches {
            let t = tokens(line);
            dw += t.len();
            if t.len() < K {
                continue;
            }
            let hits: Vec<usize> = t.windows(K).filter_map(|w| grams.get(&hash(w)).copied()).collect();
            let share = hits.len() as f64 / (t.len() - K + 1) as f64;
            if share >= 0.5 {
                kw += t.len();
                kept.push(Kept { speaker: who.clone(), text: line.clone(), chapter: hits.first().copied() });
            }
        }
        total_dw += dw;
        kept_dw += kw;
        let s = sim[i][path[i]];
        out_scenes.push(AdaptScene {
            heading: sc.heading.clone(),
            pages: sc.lines as f64 / LINES_PER_PAGE,
            chapter: (s >= 0.08).then_some(path[i]),
            score: s,
            bridged: false,
            verbatim: if dw > 0 { kw as f64 / dw as f64 } else { 0.0 },
        });
    }
    // A short unmatched run between two scenes from the same chapter is part of it.
    let mut i = 0;
    while i < out_scenes.len() {
        if out_scenes[i].chapter.is_some() {
            i += 1;
            continue;
        }
        let start = i;
        while i < out_scenes.len() && out_scenes[i].chapter.is_none() {
            i += 1;
        }
        let before = start.checked_sub(1).and_then(|k| out_scenes[k].chapter);
        let after = out_scenes.get(i).and_then(|x| x.chapter);
        let pages: f64 = out_scenes[start..i].iter().map(|x| x.pages).sum();
        if before.is_some() && before == after && pages <= 2.0 {
            for x in &mut out_scenes[start..i] {
                x.chapter = before;
                x.bridged = true;
            }
        }
    }
    let mut out_ch: Vec<AdaptChapter> = chapters.iter().map(|(t, text)| AdaptChapter { title: t.clone(), words: profile::count_words(text), scenes: vec![], pages: 0.0 }).collect();
    for (i, sc) in out_scenes.iter().enumerate() {
        if let Some(c) = sc.chapter {
            out_ch[c].scenes.push(i);
            out_ch[c].pages += sc.pages;
        }
    }
    kept.sort_by(|a, b| b.text.len().cmp(&a.text.len()));
    kept.truncate(12);
    Ok(Adaptation {
        book: book.title.clone(),
        script: script.title.clone(),
        book_words: out_ch.iter().map(|c| c.words).sum(),
        script_pages: out_scenes.iter().map(|s| s.pages).sum(),
        invented_scenes: out_scenes.iter().filter(|s| s.chapter.is_none()).count(),
        invented_pages: out_scenes.iter().filter(|s| s.chapter.is_none()).map(|s| s.pages).sum(),
        cut_chapters: out_ch.iter().filter(|c| c.scenes.is_empty()).count(),
        verbatim: if total_dw > 0 { kept_dw as f64 / total_dw as f64 } else { 0.0 },
        chapters: out_ch,
        scenes: out_scenes,
        kept,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "                               THE LONG HAUL\n\n                               Written by\n\n                              Sam Rivera\n\n          EXT. DESERT HIGHWAY - DAY\n\n          WIDE ON A RUSTED TRUCK. It sputters to a stop beside a mile\n          marker.\n\n                          NINA\n           Third time this week. I'm starting\n           to take it personally.\n\n                          OWEN (CONT'D)\n                     (from under the hood)\n           It's the \"starter.\" Probably.\n                          (CONTINUED)\n           2.\n\n          INT. TRUCK CAB - NIGHT\n\n          BOOM.\n\n          Silence.\n\n                          FADE OUT.\n";

    #[test]
    fn reads_a_screenplay() {
        let (els, title, writer) = parse(SAMPLE);
        assert_eq!(title, "THE LONG HAUL");
        assert_eq!(writer, "Sam Rivera");
        let kinds: Vec<Kind> = els.iter().map(|e| e.kind).collect();
        use Kind::*;
        assert_eq!(kinds, [Heading, Action, Cue, Dialogue, Cue, Paren, Dialogue, Heading, Action, Action]);
        assert_eq!(els[3].text, "Third time this week. I'm starting to take it personally.");
        assert_eq!(character_name(&els[4].text), "OWEN");
        let s = measure("h", "The Long Haul", &writer, "", &els);
        assert_eq!(s.scenes.len(), 2);
        assert_eq!(s.scenes[0].speeches, 2);
        assert_eq!(s.scenes[0].dialogue_words, 14);
        assert_eq!(s.scenes[1].time, "NIGHT");
        assert_eq!(s.scenes[1].place, "INT");
        assert_eq!(s.characters[0].name, "NINA");
        assert_eq!(tidy_title("THE LONG HAUL"), "The Long Haul");
    }

    #[test]
    fn writes_fountain_back() {
        let (els, _, _) = parse(SAMPLE);
        let f = fountain(&els);
        assert!(f.starts_with("EXT. DESERT HIGHWAY - DAY\n\nWIDE ON"), "{f}");
        assert!(f.contains("\n\nOWEN (CONT'D)\n(from under the hood)\nIt's the \"starter.\" Probably.\n\n"), "{f}");
        let (again, _, _) = parse(&f);
        assert_eq!(again.iter().map(|e| e.kind).collect::<Vec<_>>(), els.iter().map(|e| e.kind).collect::<Vec<_>>());
    }

    #[test]
    fn splits_headings() {
        assert_eq!(split_heading("EXT. FARM - BARN - DAY"), ("EXT".into(), "FARM - BARN".into(), "DAY".into()));
        assert_eq!(split_heading("INT./EXT. TRUCK - CONTINUOUS").0, "INT/EXT");
        assert_eq!(split_heading("INT. HOSPITAL - WAITING ROOM").2, "WAITING ROOM");
    }

    /// WP_SCRIPT=<url or file> cargo test dump_script -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dump_script() {
        let src = std::env::var("WP_SCRIPT").unwrap();
        let (els, title, writer) = parse(&load_source(&src).unwrap());
        let s = measure("x", &title, &writer, &src, &els);
        let mut j = serde_json::to_value(summarize(&s)).unwrap();
        j["pacing"] = serde_json::json!(j["pacing"].as_array().unwrap().len());
        println!("{}", serde_json::to_string_pretty(&j).unwrap());
    }
}


#[cfg(test)]
mod dump {
    /// WP_VAULT=… cargo test dump_scripts_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dump_scripts_report() {
        let v = std::path::PathBuf::from(std::env::var("WP_VAULT").unwrap());
        println!("SCRIPTS{}", serde_json::to_string(&super::report(&v)).unwrap());
        println!("ADAPT{}", serde_json::to_string(&super::adapt(&v, "the-martian", "the-martian").unwrap()).unwrap());
    }
}

#[cfg(test)]
mod dump_fountain {
    /// WP_SCRIPT=… cargo test print_fountain -- --ignored --nocapture
    #[test]
    #[ignore]
    fn print_fountain() {
        let (els, _, _) = super::parse(&super::load_source(&std::env::var("WP_SCRIPT").unwrap()).unwrap());
        print!("{}", super::fountain(&els));
    }
}
