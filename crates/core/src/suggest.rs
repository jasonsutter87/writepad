//! World entries suggested from a manuscript by plain rules, no AI.
//!
//! Screenplays are exact: character cues are the characters and scene
//! headings the locations. Prose is heuristic: capitalised names that recur
//! away from the start of a sentence (or carry a title like "Mrs."), who
//! "said" what, and places after "in/at/to…" or ending in a place word like
//! "Villa". Variants merge ("Herbert" into "Herbert White", "Mr. Crane" into
//! "Abel Crane"), and anything already in the world is left out.
//!
//! Suggestions are only a list. [`apply`] writes the ones the author picks
//! into `world/` as drafts, with an optional timeline skeleton of one event
//! per scene. [`import_csv`] brings in entries the author already keeps in a
//! spreadsheet.

use crate::{manuscript, script, world};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// "character" or "location".
    pub kind: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub mentions: usize,
    /// Block titles it appears in, in outline order.
    pub scenes: Vec<String>,
    /// Why it was suggested, e.g. "speaks 12 times" or "after in/at/to".
    pub why: String,
}

#[derive(Serialize, Debug)]
pub struct Suggestions {
    pub manuscript: String,
    pub title: String,
    pub screenplay: bool,
    pub characters: Vec<Suggestion>,
    pub locations: Vec<Suggestion>,
    /// Block titles in outline order, for a timeline skeleton.
    pub scenes: Vec<String>,
}

const TITLES: [&str; 22] = [
    "Mr.", "Mrs.", "Ms.", "Miss", "Dr.", "St.", "Sir", "Lady", "Lord", "Captain", "Sergeant-Major", "Sergeant", "Major", "Colonel", "Uncle",
    "Aunt", "Professor", "Mr", "Mrs", "Dr", "Saint", "Father",
];
/// Titles for women: a bare surname ("Fenn") isn't merged into these.
const WOMEN: [&str; 6] = ["Mrs.", "Mrs", "Ms.", "Miss", "Lady", "Aunt"];
const PLACE_WORDS: [&str; 52] = [
    "Villa", "House", "Hall", "Street", "Road", "Lane", "Avenue", "Church", "Chapel", "Cemetery", "Churchyard", "Park", "Square", "Station",
    "Junction", "Inn", "Farm", "Farmhouse", "Castle", "Abbey", "Manor", "River", "Lake", "Bridge", "Hill", "Wood", "Woods", "Forest", "Bay",
    "Island", "Mountain", "City", "Town", "Village", "Court", "Gardens", "Cottage", "Lodge", "Tower", "Works", "Mill", "Harbour", "Harbor",
    "Port", "Market", "School", "College", "Hospital", "Prison", "Asylum", "Kitchen", "Valley",
];
const PLACE_PREPS: [&str; 12] = ["in", "at", "to", "from", "into", "toward", "towards", "near", "through", "across", "beyond", "inside"];
const SPEECH: [&str; 10] = ["said", "asked", "cried", "replied", "answered", "whispered", "shouted", "called", "muttered", "demanded"];
/// Capitalised words that start sentences or exclaim, not names.
const STOP: &[&str] = &[
    "A", "An", "The", "I", "It", "He", "She", "They", "We", "You", "Me", "My", "His", "Her", "Its", "Our", "Their", "This", "That", "These",
    "Those", "There", "Here", "Then", "When", "What", "Why", "Where", "How", "Who", "Which", "If", "But", "And", "Or", "So", "Yet", "Yes", "No",
    "Not", "Oh", "Ah", "Well", "Now", "Once", "Even", "Only", "Just", "Still", "Then", "Thus", "After", "Before", "As", "At", "In", "On", "For",
    "From", "To", "Of", "With", "By", "Is", "Was", "Are", "Were", "Be", "Do", "Did", "Does", "Has", "Have", "Had", "Let", "Come", "Go", "Get",
    "Good", "God", "Thank", "Never", "Nothing", "Every", "Some", "Sometimes", "All", "One", "Two", "Three", "Four", "Five", "Six", "Seven",
    "Eight", "Nine", "Ten", "First", "Last", "Long", "Very", "Mate", "Check", "Hark", "Tut", "Sit", "Bring", "Leave", "Wish", "Like", "Up",
    "Down", "Out", "Over", "Without", "Outside", "Inside", "Neither", "Either", "Both", "Each", "Better", "Fancy", "Likely", "Might", "Must",
    "Should", "Would", "Could", "Can", "Will", "Shall", "May", "Perhaps", "Maybe", "Please", "Sorry", "Hello", "Goodbye", "Why", "Whatever",
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday", "January", "February", "March", "April", "June", "July",
    "August", "September", "October", "November", "December", "Christmas", "English", "French", "Mr.", "Mrs.", "Chapter", "Part", "Act",
    "Scene", "End", "CUT", "FADE", "INT.", "EXT.", "Continued",
];

/// Suggest characters and locations for a manuscript, skipping what the world already has.
pub fn suggest(vault: &Path, ms: &str) -> Result<Suggestions, String> {
    let m = manuscript::load(vault, ms)?;
    let blocks: Vec<(String, String)> = m
        .blocks
        .iter()
        .filter(|b| !b.raw)
        .filter_map(|b| b.variants.iter().find(|v| v.id == b.live).map(|v| (b.title.clone(), v.text.clone())))
        .collect();
    let all: String = blocks.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n\n");
    let (els, _, _) = script::parse(&all);
    let screenplay = els.iter().any(|e| e.kind == script::Kind::Heading) && els.iter().filter(|e| e.kind == script::Kind::Cue).count() >= 2;
    let (mut characters, mut locations) = if screenplay { from_script(&blocks) } else { from_prose(&blocks) };

    let known: HashSet<String> = world::entities(vault).iter().flat_map(|e| std::iter::once(e.name.to_lowercase()).chain(e.aliases.iter().map(|a| a.to_lowercase()))).collect();
    let fresh = |s: &Suggestion| !std::iter::once(&s.name).chain(&s.aliases).any(|n| known.contains(&n.to_lowercase()));
    characters.retain(fresh);
    locations.retain(fresh);
    Ok(Suggestions { manuscript: m.id, title: m.title, screenplay, characters, locations, scenes: blocks.into_iter().map(|(t, _)| t).collect() })
}

/// Per name: which block indexes it appears in, and how often.
type Tally = BTreeMap<String, BTreeMap<usize, usize>>;

fn bump(t: &mut Tally, name: &str, block: usize) {
    *t.entry(name.to_string()).or_default().entry(block).or_insert(0) += 1;
}

fn finish(kind: &str, name: String, aliases: Vec<String>, per_block: &BTreeMap<usize, usize>, titles: &[(String, String)], why: String) -> Suggestion {
    Suggestion {
        kind: kind.into(),
        name,
        aliases,
        mentions: per_block.values().sum(),
        scenes: per_block.keys().map(|&i| titles[i].0.clone()).collect(),
        why,
    }
}

/// "MRS. WHITE" -> "Mrs. White", "LABURNAM VILLA" -> "Laburnam Villa".
fn title_case(s: &str) -> String {
    const SMALL: [&str; 8] = ["and", "of", "the", "in", "on", "at", "a", "de"];
    s.split_whitespace()
        .enumerate()
        .map(|(i, w)| {
            if i > 0 && SMALL.contains(&w.to_lowercase().as_str()) {
                return w.to_lowercase();
            }
            w.split('-')
                .map(|p| {
                    let mut c = p.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn from_script(blocks: &[(String, String)]) -> (Vec<Suggestion>, Vec<Suggestion>) {
    let (mut speakers, mut places): (Tally, Tally) = (Tally::new(), Tally::new());
    for (i, (_, text)) in blocks.iter().enumerate() {
        for e in script::parse(text).0 {
            match e.kind {
                script::Kind::Cue => {
                    let n = script::character_name(&e.text);
                    if !n.is_empty() {
                        bump(&mut speakers, &n, i);
                    }
                }
                script::Kind::Heading => {
                    let (_, loc, _) = script::split_heading(&e.text);
                    // "WRIGHT FARMHOUSE - KITCHEN": the first part is the place; the rest is a room in it.
                    let place = loc.split(" - ").next().unwrap_or("").trim().to_string();
                    if !place.is_empty() {
                        bump(&mut places, &place, i);
                    }
                }
                _ => {}
            }
        }
    }
    let mut chars: Vec<Suggestion> = speakers
        .iter()
        .map(|(cue, per)| {
            let n: usize = per.values().sum();
            finish("character", title_case(cue), vec![cue.clone()], per, blocks, format!("speaks {n} time{}", if n == 1 { "" } else { "s" }))
        })
        .collect();
    let mut locs: Vec<Suggestion> = places
        .iter()
        .map(|(p, per)| {
            let n: usize = per.values().sum();
            finish("location", title_case(p), vec![p.clone()], per, blocks, format!("{n} scene heading{}", if n == 1 { "" } else { "s" }))
        })
        .collect();
    chars.sort_by(|a, b| b.mentions.cmp(&a.mentions).then(a.name.cmp(&b.name)));
    locs.sort_by(|a, b| b.mentions.cmp(&a.mentions).then(a.name.cmp(&b.name)));
    (chars, locs)
}

/// One capitalised run in prose, with what we know about where it sat.
struct Hit {
    name: String,
    block: usize,
    /// First word of a sentence (or of a quotation), where any word is capitalised.
    sentence_start: bool,
    titled: bool,
    after_place_prep: bool,
    speaker: bool,
}

const OPEN: &[char] = &['“', '"', '‘', '\'', '(', '[', '—', '-', '*', '_'];
const CLOSE: &[char] = &['”', '"', '’', '\'', ')', ']', ',', '.', ';', ':', '!', '?', '—', '*', '_'];

fn is_cap(w: &str) -> bool {
    let mut c = w.chars();
    matches!(c.next(), Some(f) if f.is_uppercase()) && w.chars().any(|c| c.is_lowercase())
}

/// "Fenn's" -> "Fenn"; leaves "It's" for the stop list to catch.
fn strip_possessive(w: &str) -> &str {
    w.strip_suffix("’s").or_else(|| w.strip_suffix("'s")).unwrap_or(w)
}

fn prose_hits(blocks: &[(String, String)]) -> Vec<Hit> {
    let mut hits = vec![];
    for (bi, (_, text)) in blocks.iter().enumerate() {
        for para in text.split("\n\n") {
            let words: Vec<&str> = para.split_whitespace().collect();
            let mut i = 0;
            let mut sentence_start = true;
            while i < words.len() {
                let raw = words[i];
                let opens_quote = raw.starts_with(['“', '"', '‘']);
                let core = raw.trim_start_matches(OPEN);
                let ends_sentence = |w: &str| w.trim_end_matches(['”', '"', '’', '\'', ')', '*', '_']).ends_with(['.', '!', '?']);
                let is_title = TITLES.contains(&core.trim_end_matches([',', ';', ':'])) || TITLES.contains(&core);
                if !(is_cap(core) || is_title) {
                    sentence_start = ends_sentence(raw) && !TITLES.contains(&core);
                    i += 1;
                    continue;
                }
                // "Then Herbert", "But Mr. Crane", "It's Herbert": the stop word only starts the
                // sentence. Contractions ("I'm", "He'd") are never names.
                let word = core.trim_end_matches(CLOSE);
                let contraction = word.contains(['’', '\'']) && strip_possessive(word) == word;
                if !is_title && (contraction || STOP.contains(&strip_possessive(word))) {
                    sentence_start = ends_sentence(raw);
                    i += 1;
                    continue;
                }
                // Gather the run: Title? Cap (of|de|van|von|la)? Cap ...
                let start = i;
                let starts_sentence = sentence_start || opens_quote;
                let mut parts: Vec<String> = vec![];
                let mut titled = false;
                loop {
                    let w = words[i].trim_start_matches(OPEN);
                    let bare = w.trim_end_matches(CLOSE);
                    let t = if TITLES.contains(&w) { w } else { bare };
                    if TITLES.contains(&t) && parts.is_empty() {
                        titled = true;
                        parts.push(t.to_string());
                    } else if is_cap(bare) {
                        // "St. Aldric's" is the church's name; "Fenn's" is Fenn's.
                        let saint = matches!(parts.first().map(String::as_str), Some("St." | "Saint"));
                        let base = if saint { bare } else { strip_possessive(bare) };
                        parts.push(base.to_string());
                        if base.len() != bare.len() {
                            i += 1; // "Fenn's grave": the possessive ends the name
                            break;
                        }
                    } else {
                        break;
                    }
                    // A run stops at punctuation (except a title's own dot).
                    let stops = !TITLES.contains(&w) && w.len() != bare.len();
                    i += 1;
                    if stops || i >= words.len() {
                        break;
                    }
                    let next = words[i];
                    if ["of", "de", "van", "von", "la", "le", "du"].contains(&next) && i + 1 < words.len() && is_cap(words[i + 1].trim_start_matches(OPEN)) {
                        parts.push(next.to_string());
                        i += 1;
                    }
                }
                let last_raw = words[i.saturating_sub(1)];
                sentence_start = ends_sentence(last_raw) && !TITLES.contains(&last_raw.trim_start_matches(OPEN));
                if i == start {
                    i += 1;
                    continue;
                }
                // A title alone ("Mr.") isn't a name.
                if parts.len() == 1 && titled {
                    continue;
                }
                let name = parts.join(" ");
                if name.chars().count() < 3 || name.contains('’') && !name.ends_with(['s']) {
                    continue;
                }
                let prev = if start > 0 { words[start - 1].trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase() } else { String::new() };
                let next = words.get(i).map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).unwrap_or_default();
                let prev2 = if start > 1 { words[start - 2].trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase() } else { String::new() };
                hits.push(Hit {
                    after_place_prep: PLACE_PREPS.contains(&prev.as_str()) || (prev == "the" && PLACE_PREPS.contains(&prev2.as_str())),
                    speaker: SPEECH.contains(&prev.as_str()) || SPEECH.contains(&next.as_str()),
                    name,
                    block: bi,
                    // Two capitalised words together mean a name even at a sentence start.
                    sentence_start: starts_sentence && !titled && parts.len() == 1,
                    titled,
                });
            }
        }
    }
    hits
}

fn from_prose(blocks: &[(String, String)]) -> (Vec<Suggestion>, Vec<Suggestion>) {
    struct Stat {
        per: BTreeMap<usize, usize>,
        mid: usize,
        titled: bool,
        place_prep: usize,
        speaker: usize,
    }
    let mut stats: BTreeMap<String, Stat> = BTreeMap::new();
    for h in prose_hits(blocks) {
        let s = stats.entry(h.name.clone()).or_insert(Stat { per: BTreeMap::new(), mid: 0, titled: false, place_prep: 0, speaker: 0 });
        *s.per.entry(h.block).or_insert(0) += 1;
        s.mid += usize::from(!h.sentence_start);
        s.titled |= h.titled;
        s.place_prep += usize::from(h.after_place_prep);
        s.speaker += usize::from(h.speaker);
    }
    // Merge variants into the fullest form: first names and surnames into "First Last",
    // and bare surnames into a unique man's titled name ("Morris" -> "Sergeant-Major Morris").
    // "Mr. White" never merges into "Herbert White": families share surnames, and a
    // wrong merge is worse than a duplicate the author can fold in.
    let names: Vec<String> = stats.keys().cloned().collect();
    let split_title = |n: &str| -> (Option<String>, String) {
        let mut w = n.split(' ');
        let first = w.next().unwrap_or("");
        if TITLES.contains(&first) {
            (Some(first.to_string()), w.collect::<Vec<_>>().join(" "))
        } else {
            (None, n.to_string())
        }
    };
    let mut into: HashMap<String, String> = HashMap::new();
    for n in &names {
        if n.contains(' ') {
            continue;
        }
        let candidates: Vec<&String> = names
            .iter()
            .filter(|full| *full != n)
            .filter(|full| {
                let (title, rest) = split_title(full);
                let ws: Vec<&str> = rest.split(' ').collect();
                match title {
                    // "Mr. Crane" / "Sergeant-Major Morris": the bare surname, unless the title is a woman's.
                    Some(t) => ws.last() == Some(&n.as_str()) && !WOMEN.contains(&t.as_str()) && ws.len() == 1,
                    // "Herbert White" / "Abel Crane": the first name or the surname.
                    None => ws.len() >= 2 && (ws[0] == n || ws.last() == Some(&n.as_str())),
                }
            })
            .collect();
        if candidates.len() == 1 {
            into.insert(n.clone(), candidates[0].clone());
        }
    }
    let root = |n: &String| -> String {
        let mut r = n.clone();
        for _ in 0..4 {
            match into.get(&r) {
                Some(next) => r = next.clone(),
                None => break,
            }
        }
        r
    };
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in &names {
        groups.entry(root(n)).or_default().push(n.clone());
    }

    let (mut chars, mut locs) = (vec![], vec![]);
    for (name, members) in groups {
        let mut per: BTreeMap<usize, usize> = BTreeMap::new();
        let (mut titled, mut place_prep, mut speaker, mut mid) = (false, 0, 0, 0);
        for m in &members {
            let s = &stats[m];
            mid += s.mid;
            for (b, c) in &s.per {
                *per.entry(*b).or_insert(0) += c;
            }
            titled |= s.titled;
            place_prep += s.place_prep;
            speaker += s.speaker;
        }
        let mentions: usize = per.values().sum();
        let last = name.split(' ').next_back().unwrap_or("");
        let saint = name.starts_with("St. ") || name.starts_with("Saint ");
        // Recurring, and somewhere a capital means something: mid-sentence, with a title, or as a
        // speaker. A place-shaped name ("Laburnam Villa") counts from one mention.
        let place_shaped = (PLACE_WORDS.contains(&last) || saint) && name.contains(' ');
        if !(mentions >= 2 || place_shaped) || !(mid > 0 || titled || speaker > 0 || place_shaped) {
            continue;
        }
        let aliases: Vec<String> = members.into_iter().filter(|m| *m != name).collect();
        let place = PLACE_WORDS.contains(&last) || saint || (place_prep * 2 >= mentions && speaker == 0 && !titled);
        if place {
            let why = if PLACE_WORDS.contains(&last) || saint { "named like a place".to_string() } else { format!("after in/at/to {place_prep} times") };
            locs.push(finish("location", name, aliases, &per, blocks, why));
        } else {
            let why = if speaker > 0 {
                format!("speaks ({speaker} “said” lines)")
            } else if titled {
                "has a title".to_string()
            } else {
                format!("named {mentions} times")
            };
            chars.push(finish("character", name, aliases, &per, blocks, why));
        }
    }
    chars.sort_by(|a, b| b.mentions.cmp(&a.mentions).then(a.name.cmp(&b.name)));
    locs.sort_by(|a, b| b.mentions.cmp(&a.mentions).then(a.name.cmp(&b.name)));
    (chars, locs)
}

/// A note name the vault can store: no path separators or colons.
fn note_name(s: &str) -> String {
    s.replace([':', '/', '\\'], " -").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Write the picked suggestions into the world as drafts, plus (optionally) a
/// timeline skeleton of one event per scene. Returns the notes created.
pub fn apply(vault: &Path, ms: &str, picks: &[Suggestion], timeline: bool) -> Result<Vec<String>, String> {
    let m = manuscript::manifest(vault, ms)?;
    let draft = |extra: Vec<(String, String)>| -> Vec<(String, String)> {
        let mut p = vec![("status".to_string(), "draft".to_string()), ("drafted_by".to_string(), "\"suggest\"".to_string())];
        p.extend(extra);
        p
    };
    let mut created = vec![];
    // If the world is scoped to other manuscripts, names here wouldn't be matched; scope one in.
    let es = world::entities(vault);
    let scoped: Vec<String> = es.iter().filter(|e| e.kind == "world").flat_map(|e| e.props["manuscripts"].as_array().cloned().unwrap_or_default()).filter_map(|v| v.as_str().map(String::from)).collect();
    if !scoped.is_empty() && !scoped.iter().any(|t| t == &m.title) && !es.iter().any(|e| e.name == note_name(&m.title)) {
        let props = draft(vec![("manuscripts".into(), serde_json::to_string(&[&m.title]).unwrap())]);
        created.push(world::add(vault, "world", &note_name(&m.title), &props, &format!("# {}\n\nThe world of *{}*.", m.title, m.title))?);
    }
    for s in picks {
        let kind = if s.kind == "location" { "location" } else { "character" };
        let mut extra = vec![];
        if !s.aliases.is_empty() {
            extra.push(("aliases".to_string(), serde_json::to_string(&s.aliases).unwrap()));
        }
        let scenes: Vec<String> = s.scenes.iter().map(|t| format!("[[{}#{}]]", m.title, t)).collect();
        let body = format!(
            "# {}\n\nSuggested from *{}*: named {} time{} ({}). Appears in {}.",
            s.name,
            m.title,
            s.mentions,
            if s.mentions == 1 { "" } else { "s" },
            s.why,
            scenes.join(", ")
        );
        match world::add(vault, kind, &note_name(&s.name), &draft(extra), &body) {
            Ok(p) => created.push(p),
            Err(e) if e.ends_with("already exists.") => {}
            Err(e) => return Err(e),
        }
    }
    if timeline {
        let mut prev: Option<String> = None;
        let mut n = 0;
        for b in m.blocks.iter().filter(|b| !b.raw) {
            n += 1;
            let name = note_name(&format!("{} · {n}. {}", m.title, b.title));
            let props = draft(vec![("scenes".into(), serde_json::to_string(&[format!("[[{}#{}]]", m.title, b.title)]).unwrap())]);
            let body = format!("What happens in [[{}#{}]]. Rename this event, or split it into several.", m.title, b.title);
            match world::insert_event(vault, &name, prev.as_deref(), None, props, &body) {
                Ok(p) => {
                    created.push(p);
                    prev = Some(name);
                }
                Err(e) if e.ends_with("already exists.") => prev = Some(name),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(created)
}

/// What a CSV import did.
#[derive(Serialize, Debug, Default)]
pub struct CsvImport {
    pub created: Vec<String>,
    /// "row 4: Unknown type \"monster\"" and the like.
    pub skipped: Vec<String>,
}

/// Import world entries from a CSV with a header row: `type,name,aliases,description`
/// (aliases separated by `;`; columns in any order; `kind` works for `type`).
/// Types: character (person), location (place), thing (item, object).
/// These are the author's own notes, so they aren't marked as drafts.
pub fn import_csv(vault: &Path, text: &str) -> Result<CsvImport, String> {
    let rows = parse_csv(text.trim_start_matches('\u{feff}'));
    let header: Vec<String> = rows.first().ok_or("The CSV is empty.")?.iter().map(|h| h.trim().to_lowercase()).collect();
    let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let (ty, name) = (col(&["type", "kind"]).ok_or("The CSV needs a \"type\" column.")?, col(&["name"]).ok_or("The CSV needs a \"name\" column.")?);
    let (aliases, desc) = (col(&["aliases", "alias", "also known as"]), col(&["description", "notes", "summary", "body"]));
    let mut out = CsvImport::default();
    for (i, row) in rows.iter().enumerate().skip(1) {
        let cell = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|s| s.trim().to_string()).unwrap_or_default();
        let n = note_name(&cell(Some(name)));
        if n.is_empty() {
            continue;
        }
        let kind = match cell(Some(ty)).to_lowercase().as_str() {
            "character" | "person" | "people" | "char" => "character",
            "location" | "place" | "setting" => "location",
            "thing" | "item" | "object" | "prop" => "thing",
            other => {
                out.skipped.push(format!("row {}: unknown type {other:?} (use character, location or thing)", i + 1));
                continue;
            }
        };
        let al: Vec<String> = cell(aliases).split(';').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
        let props = if al.is_empty() { vec![] } else { vec![("aliases".to_string(), serde_json::to_string(&al).unwrap())] };
        let d = cell(desc);
        let body = if d.is_empty() { format!("# {n}") } else { format!("# {n}\n\n{d}") };
        match world::add(vault, kind, &n, &props, &body) {
            Ok(p) => out.created.push(p),
            Err(e) => out.skipped.push(format!("row {}: {e}", i + 1)),
        }
    }
    Ok(out)
}

/// RFC 4180-ish: quoted fields with "" escapes, commas and newlines inside quotes.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (vec![], vec![], String::new());
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(parts: &[(&str, &str)]) -> Vec<(String, String)> {
        parts.iter().map(|(t, x)| (t.to_string(), x.to_string())).collect()
    }

    #[test]
    fn prose_finds_and_merges_names() {
        let b = blocks(&[
            ("One", "Abel Crane had kept the place thirty years. “This is the bell-row,” said Abel.\n\nThen Mr. Crane walked to St. Aldric's with Tom. Mrs. Fenn watched. “Yes,” said Tom."),
            ("Two", "Abel laughed. Mr. Crane spat. Tom ran to St. Aldric's in the snow. “I'm here,” said Mrs. Fenn. It was February. “It's Herbert!” Fenn's grave lay by Laburnam Villa."),
        ]);
        let (chars, locs) = from_prose(&b);
        let abel = chars.iter().find(|c| c.name == "Abel Crane").unwrap_or_else(|| panic!("no Abel Crane in {chars:#?}
{locs:#?}"));
        assert!(abel.aliases.contains(&"Abel".to_string()), "{:?}", abel.aliases);
        // Titled names stay separate from full names (they're often a relative).
        assert!(chars.iter().any(|c| c.name == "Mr. Crane"));
        assert_eq!(abel.scenes, vec!["One", "Two"]);
        assert!(chars.iter().any(|c| c.name == "Tom"));
        assert!(chars.iter().any(|c| c.name == "Mrs. Fenn"));
        assert!(locs.iter().any(|l| l.name == "St. Aldric's"), "{locs:?}");
        assert!(locs.iter().any(|l| l.name == "Laburnam Villa"), "{locs:?}");
        let all: Vec<&str> = chars.iter().chain(&locs).map(|s| s.name.as_str()).collect();
        for noise in ["Then", "February", "Yes", "I'm", "It", "It Herbert"] {
            assert!(!all.contains(&noise), "{noise} in {all:?}");
        }
    }

    #[test]
    fn screenplays_use_cues_and_headings() {
        let b = blocks(&[
            ("Act One", "INT. WRIGHT FARMHOUSE - KITCHEN - DAY\n\nThe kitchen.\n\nSHERIFF\nIt's just the same.\n\nMRS HALE\n(quietly)\nI know.\n\nSHERIFF\nWell."),
            ("Act Two", "EXT. WRIGHT FARMHOUSE - DAY\n\nSnow.\n\nMRS HALE\nCome on."),
        ]);
        let (chars, locs) = from_script(&b);
        assert_eq!(chars.iter().map(|c| (c.name.as_str(), c.mentions)).collect::<Vec<_>>(), vec![("Mrs Hale", 2), ("Sheriff", 2)]);
        assert_eq!(locs[0].name, "Wright Farmhouse");
        assert_eq!(locs[0].scenes, vec!["Act One", "Act Two"]);
    }

    #[test]
    fn csv_parses_quotes_and_rows() {
        let rows = parse_csv("type,name,aliases,description\ncharacter,\"Crane, Abel\",Abel;Mr. Crane,\"Said \"\"rat\"\".\"\n\nlocation,St. Aldric's,,\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["character", "Crane, Abel", "Abel;Mr. Crane", "Said \"rat\"."]);
        assert_eq!(rows[2], vec!["location", "St. Aldric's", "", ""]);
    }

    #[test]
    fn csv_import_writes_notes() {
        let v = std::env::temp_dir().join(format!("wp-csv-{}", std::process::id()));
        std::fs::create_dir_all(&v).unwrap();
        let r = import_csv(&v, "Name,Type,Aliases\nJune,person,Junie\nUnit 12,place,\nThe van,vehicle,\n").unwrap();
        assert_eq!(r.created, vec!["world/characters/June.md", "world/locations/Unit 12.md"]);
        assert_eq!(r.skipped.len(), 1);
        assert!(std::fs::read_to_string(v.join("world/characters/June.md")).unwrap().contains("aliases: [\"Junie\"]"));
        std::fs::remove_dir_all(&v).unwrap();
    }
}
