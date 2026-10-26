//! World: the story's places, people, relationships and timeline, kept as
//! ordinary markdown notes under `world/` (Obsidian-compatible frontmatter),
//! and the knowledge graph built from them.
//!
//! ```text
//! world/World.md                     type: world (the overview)
//! world/locations/Unit 12.md         type: location
//! world/characters/June.md           type: character, aliases, relationships
//! world/relationships/June & Dolores.md   type: relationship, between, kind
//! world/timeline/The van.md          type: event, after: "[[previous event]]", scenes
//! ```
//!
//! The type comes from the `type:` property, or else from the folder. Edges
//! come from wikilinks, `relationships:` lists, relationship notes, timeline
//! `after:` links, event `scenes:`, and names/aliases found in manuscripts.
//!
//! The timeline is a linked list: each event points at the one before it with
//! `after:`. Two events after the same one are a branch.

use crate::manuscript;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub const FOLDER: &str = "world";

/// Where new notes of each type go, and the order types are shown in.
pub const TYPES: [(&str, &str); 6] = [
    ("world", ""),
    ("location", "locations"),
    ("character", "characters"),
    ("relationship", "relationships"),
    ("event", "timeline"),
    ("thing", "things"),
];

#[derive(Serialize, Clone, Debug)]
pub struct Relation {
    /// Raw link target (note name).
    pub to: String,
    pub kind: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Entity {
    /// Vault-relative path.
    pub path: String,
    /// File name without .md: what [[links]] use.
    pub name: String,
    pub kind: String,
    pub aliases: Vec<String>,
    /// First paragraph of the body, for previews.
    pub summary: String,
    /// All frontmatter properties.
    pub props: Value,
    pub links: Vec<String>,
    pub relations: Vec<Relation>,
    pub after: Vec<String>,
    pub when: Option<String>,
    pub scenes: Vec<String>,
    /// e.g. "draft" for notes an agent proposed.
    pub status: Option<String>,
}

/// Split `---\nyaml\n---\nbody`. No frontmatter -> (None, whole text).
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return (None, text) };
    for (i, _) in rest.match_indices("\n---") {
        let after = &rest[i + 4..];
        if after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n") {
            let body = after.trim_start_matches(['\r', '\n']);
            return (Some(&rest[..i]), body);
        }
    }
    (None, text)
}

/// `[[Target|alias]]` / `[[Target#Heading]]` -> "Target" (or "Target#Heading" with `keep_heading`).
fn link_target(inner: &str, keep_heading: bool) -> String {
    let t = inner.split('|').next().unwrap_or(inner);
    let t = if keep_heading { t } else { t.split('#').next().unwrap_or(t) };
    t.trim().trim_end_matches(".md").to_string()
}

/// `[[Target|alias]]` -> "alias", `[[Target]]` -> "Target", for display.
fn plain_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(a) = rest.find("[[") {
        let Some(b) = rest[a..].find("]]") else { break };
        out.push_str(&rest[..a]);
        let inner = &rest[a + 2..a + b];
        out.push_str(inner.rsplit('|').next().unwrap_or(inner));
        rest = &rest[a + b + 2..];
    }
    out.push_str(rest);
    out
}

pub fn wikilinks(text: &str, keep_heading: bool) -> Vec<String> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(a) = rest.find("[[") {
        let Some(b) = rest[a..].find("]]") else { break };
        let t = link_target(&rest[a + 2..a + b], keep_heading);
        if !t.is_empty() {
            out.push(t);
        }
        rest = &rest[a + b + 2..];
    }
    out
}

/// A property value as link targets: "[[X]]", "X", or a list of those.
fn targets(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => {
            let links = wikilinks(s, true);
            if links.is_empty() && !s.trim().is_empty() { vec![s.trim().to_string()] } else { links }
        }
        Some(Value::Array(a)) => a.iter().flat_map(|x| targets(Some(x))).collect(),
        _ => vec![],
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

fn kind_from_folder(rel: &str) -> String {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() >= 3 {
        if let Some((k, _)) = TYPES.iter().find(|(_, f)| !f.is_empty() && *f == parts[1]) {
            return k.to_string();
        }
    }
    if parts.len() == 2 && parts[1].eq_ignore_ascii_case("world.md") {
        return "world".into();
    }
    "note".into()
}

/// The first paragraph of prose; failing that, the first heading's text.
fn summary_of(body: &str) -> String {
    let mut para: Vec<&str> = Vec::new();
    let mut heading = None;
    for line in body.lines().map(str::trim) {
        if let Some(h) = line.strip_prefix('#') {
            heading.get_or_insert_with(|| h.trim_start_matches('#').trim());
            if !para.is_empty() {
                break;
            }
        } else if line.is_empty() {
            if !para.is_empty() {
                break;
            }
        } else {
            para.push(line);
        }
    }
    if para.is_empty() { heading.unwrap_or("").to_string() } else { para.join(" ") }
}

pub fn parse(rel: &str, text: &str) -> Entity {
    let (fm, body) = split_frontmatter(text);
    let props: Value = fm
        .and_then(|y| serde_yaml::from_str::<serde_yaml::Value>(y).ok())
        .and_then(|y| serde_json::to_value(y).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let name = Path::new(rel).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = props["type"].as_str().map(|s| s.trim().to_lowercase()).unwrap_or_else(|| kind_from_folder(rel));

    let mut relations = vec![];
    if let Some(list) = props["relationships"].as_array() {
        for r in list {
            match r {
                Value::Object(_) => {
                    for to in targets(r.get("to")) {
                        relations.push(Relation { to, kind: r["kind"].as_str().unwrap_or("related to").to_string() });
                    }
                }
                other => {
                    for to in targets(Some(other)) {
                        relations.push(Relation { to, kind: "related to".into() });
                    }
                }
            }
        }
    }
    let summary = plain_links(&summary_of(body)).chars().take(280).collect();
    let mut links = wikilinks(body, false);
    // Frontmatter "Book#Section" links are scene refs, not links to notes.
    links.extend(wikilinks(fm.unwrap_or(""), true).into_iter().filter(|l| !l.contains('#')));
    links.sort();
    links.dedup();

    Entity {
        path: rel.to_string(),
        kind,
        aliases: strings(props.get("aliases")),
        summary,
        links,
        relations,
        after: targets(props.get("after")),
        when: props["when"].as_str().map(String::from).or_else(|| props["when"].as_i64().map(|n| n.to_string())),
        scenes: targets(props.get("scenes")),
        status: props["status"].as_str().map(String::from),
        props,
        name,
    }
}

/// Every note in the vault (not just `world/`), parsed. Notes outside
/// `world/` join the graph only if they declare a `type:`.
pub fn entities(vault: &Path) -> Vec<Entity> {
    crate::vault::list_notes(vault)
        .into_iter()
        .filter_map(|n| {
            let text = fs::read_to_string(vault.join(&n.path)).ok()?;
            let e = parse(&n.path, &text);
            let in_world = n.path.starts_with(&format!("{FOLDER}/"));
            (in_world || e.props.get("type").is_some()).then_some(e)
        })
        .collect()
}

/// Resolves link targets (names, aliases, paths) to entity paths.
struct Index<'a> {
    by_key: HashMap<String, &'a Entity>,
}

impl<'a> Index<'a> {
    fn new(es: &'a [Entity]) -> Self {
        let mut by_key = HashMap::new();
        for e in es {
            for k in std::iter::once(&e.name).chain(&e.aliases) {
                by_key.entry(k.to_lowercase()).or_insert(e);
            }
            by_key.insert(e.path.trim_end_matches(".md").to_lowercase(), e);
        }
        Index { by_key }
    }
    fn get(&self, target: &str) -> Option<&'a Entity> {
        let t = target.split('#').next().unwrap_or(target).trim().to_lowercase();
        self.by_key.get(&t).copied()
    }
}

#[derive(Serialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub summary: String,
    /// For scene nodes: where to open them.
    pub manuscript: Option<String>,
    pub block: Option<String>,
}

#[derive(Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// "link" | "relation" | "next" | "scene" | "mention"
    pub kind: String,
    pub label: String,
    pub weight: usize,
}

#[derive(Serialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub issues: Vec<Issue>,
}

/// A block of a manuscript, by its live take.
struct Scene {
    id: String,
    label: String,
    ms: String,
    ms_title: String,
    block: String,
    title: String,
    text: String,
    /// Position of the block in its manuscript.
    index: usize,
}

/// The manuscripts this world belongs to: `manuscripts:` on the world
/// overview note (titles or ids). Empty means every manuscript in the vault.
fn scope(es: &[Entity]) -> Vec<String> {
    es.iter()
        .filter(|e| e.kind == "world")
        .flat_map(|e| strings(e.props.get("manuscripts")))
        .map(|m| m.trim_start_matches("[[").trim_end_matches("]]").to_lowercase())
        .collect()
}

fn scenes_in(vault: &Path, es: &[Entity]) -> Vec<Scene> {
    let only = scope(es);
    scenes(vault)
        .into_iter()
        .filter(|s| only.is_empty() || only.contains(&s.ms) || only.contains(&s.ms_title.to_lowercase()))
        .collect()
}

fn scenes(vault: &Path) -> Vec<Scene> {
    let mut out = vec![];
    for info in manuscript::list(vault) {
        let Ok(m) = manuscript::load(vault, &info.id) else { continue };
        for (index, b) in m.blocks.iter().enumerate() {
            let text = b.variants.iter().find(|v| v.id == b.live).or(b.variants.last()).map(|v| v.text.clone()).unwrap_or_default();
            out.push(Scene {
                id: format!("scene:{}/{}", m.id, b.id),
                label: format!("{} · {}", m.title, b.title),
                ms: m.id.clone(),
                ms_title: m.title.clone(),
                block: b.id.clone(),
                title: b.title.clone(),
                text,
                index,
            });
        }
    }
    out
}

/// Whole-word, case-sensitive occurrences of `name` in `text`.
fn count_mentions(text: &str, name: &str) -> usize {
    if name.chars().count() < 3 {
        return 0;
    }
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    // A character called June (or April, May…) isn't in "June 13".
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let is_month = MONTHS.contains(&name);
    text.match_indices(name)
        .filter(|(i, _)| boundary(text[..*i].chars().next_back()) && boundary(text[i + name.len()..].chars().next()))
        .filter(|(i, _)| {
            if !is_month {
                return true;
            }
            let after = text[i + name.len()..].trim_start_matches([' ', '\u{a0}']);
            !after.starts_with(|c: char| c.is_ascii_digit())
        })
        .count()
}

/// Kinds whose names are looked for in the manuscript text.
fn mentionable(kind: &str) -> bool {
    !matches!(kind, "event" | "relationship" | "world" | "note")
}

/// Resolve "Manuscript Title#Block title" (or id#id) to a scene.
fn find_scene<'a>(scenes: &'a [Scene], target: &str) -> Option<&'a Scene> {
    let (ms, block) = target.split_once('#')?;
    let (ms, block) = (ms.trim().to_lowercase(), block.trim().to_lowercase());
    scenes.iter().find(|s| {
        (s.ms_title.to_lowercase() == ms || s.ms == ms) && (s.title.to_lowercase() == block || s.block == block)
    })
}

pub fn graph(vault: &Path) -> Graph {
    let es = entities(vault);
    let idx = Index::new(&es);
    let scenes = scenes_in(vault, &es);
    let mut nodes: Vec<Node> = es
        .iter()
        .map(|e| Node { id: e.path.clone(), label: e.name.clone(), kind: e.kind.clone(), summary: e.summary.clone(), manuscript: None, block: None })
        .collect();
    let mut edges: Vec<Edge> = vec![];
    let mut issues: Vec<Issue> = vec![];
    let mut used_scenes: HashSet<String> = HashSet::new();
    // Linked names with no note yet: a to-do list for the world.
    let mut missing: std::collections::BTreeMap<String, String> = Default::default();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut push = |edges: &mut Vec<Edge>, from: &str, to: &str, kind: &str, label: &str, weight: usize| {
        if from != to && seen.insert((from.to_string(), to.to_string(), format!("{kind}:{label}"))) {
            edges.push(Edge { from: from.into(), to: to.into(), kind: kind.into(), label: label.into(), weight });
        }
    };

    for e in &es {
        for r in &e.relations {
            match idx.get(&r.to) {
                Some(t) => push(&mut edges, &e.path, &t.path, "relation", &r.kind, 1),
                None => issues.push(Issue { path: e.path.clone(), message: format!("Relationship to [[{}]], which has no note yet.", r.to) }),
            }
        }
        if e.kind == "relationship" {
            let between = targets(e.props.get("between"));
            let kind = e.props["kind"].as_str().unwrap_or("related to");
            let ends: Vec<&Entity> = between.iter().filter_map(|t| idx.get(t)).collect();
            if ends.len() == 2 {
                push(&mut edges, &ends[0].path, &ends[1].path, "relation", kind, 1);
                push(&mut edges, &e.path, &ends[0].path, "link", "", 1);
                push(&mut edges, &e.path, &ends[1].path, "link", "", 1);
            }
        }
        for prev in &e.after {
            match idx.get(prev) {
                Some(p) => push(&mut edges, &p.path, &e.path, "next", "then", 1),
                None => issues.push(Issue { path: e.path.clone(), message: format!("Comes after [[{prev}]], which isn't an event note.") }),
            }
        }
        for s in &e.scenes {
            match find_scene(&scenes, s) {
                Some(sc) => {
                    used_scenes.insert(sc.id.clone());
                    push(&mut edges, &e.path, &sc.id, "scene", "happens in", 1);
                }
                None => issues.push(Issue { path: e.path.clone(), message: format!("Scene [[{s}]] wasn't found. Use \"Manuscript title#Block title\".") }),
            }
        }
        for l in &e.links {
            match idx.get(l) {
                Some(t) => push(&mut edges, &e.path, &t.path, "link", "", 1),
                None if e.path.starts_with(&format!("{FOLDER}/"))
                    && !missing.contains_key(l)
                    && !scenes.iter().any(|sc| sc.ms_title.eq_ignore_ascii_case(l)) =>
                {
                    missing.insert(l.clone(), e.name.clone());
                }
                None => {}
            }
        }
    }

    // Who and where appears in which scene.
    for e in es.iter().filter(|e| mentionable(&e.kind)) {
        let names: Vec<&String> = std::iter::once(&e.name).chain(&e.aliases).collect();
        let mut total = 0;
        for sc in &scenes {
            let n: usize = names.iter().map(|name| count_mentions(&sc.text, name)).sum();
            if n > 0 {
                total += n;
                used_scenes.insert(sc.id.clone());
                push(&mut edges, &e.path, &sc.id, "mention", "appears in", n);
            }
        }
        if total == 0 && !scenes.is_empty() {
            issues.push(Issue { path: e.path.clone(), message: format!("{} never appears in a manuscript (by name or alias).", e.name) });
        }
    }
    for (name, from) in &missing {
        issues.push(Issue { path: String::new(), message: format!("[[{name}]] is linked from {from} but has no note yet.") });
    }
    for sc in scenes.iter().filter(|s| used_scenes.contains(&s.id)) {
        nodes.push(Node {
            id: sc.id.clone(),
            label: sc.label.clone(),
            kind: "scene".into(),
            summary: sc.text.chars().take(200).collect(),
            manuscript: Some(sc.ms.clone()),
            block: Some(sc.block.clone()),
        });
    }
    Graph { nodes, edges, issues }
}

/// Everything about one manuscript block: who and what appears in it, and
/// which timeline events happen in it.
#[derive(Serialize)]
pub struct SceneInfo {
    pub present: Vec<(Entity, usize)>,
    pub events: Vec<Entity>,
}

pub fn scene_info(vault: &Path, ms: &str, block: &str) -> Result<SceneInfo, String> {
    let m = manuscript::load(vault, ms)?;
    let b = m.blocks.iter().find(|b| b.id == block).ok_or("No such block")?;
    let text = b.variants.iter().find(|v| v.id == b.live).or(b.variants.last()).map(|v| v.text.as_str()).unwrap_or("");
    let es = entities(vault);
    let mut present: Vec<(Entity, usize)> = es
        .iter()
        .filter(|e| mentionable(&e.kind))
        .filter_map(|e| {
            let n: usize = std::iter::once(&e.name).chain(&e.aliases).map(|name| count_mentions(text, name)).sum();
            (n > 0).then(|| (e.clone(), n))
        })
        .collect();
    present.sort_by(|a, b| b.1.cmp(&a.1));
    let key_title = format!("{}#{}", m.title, b.title).to_lowercase();
    let key_id = format!("{}#{}", m.id, b.id).to_lowercase();
    let events = es
        .into_iter()
        .filter(|e| e.kind == "event" && e.scenes.iter().any(|s| s.to_lowercase() == key_title || s.to_lowercase() == key_id))
        .collect();
    Ok(SceneInfo { present, events })
}

/// One entity with everything that points at it.
#[derive(Serialize)]
pub struct EntityInfo {
    pub entity: Entity,
    /// (other entity's path, name, relation kind, outgoing?)
    pub relations: Vec<(String, String, String, bool)>,
    /// Notes that link here.
    pub backlinks: Vec<(String, String)>,
    /// (manuscript id, block id, label, mentions)
    pub appears_in: Vec<(String, String, String, usize)>,
}

pub fn entity_info(vault: &Path, path: &str) -> Result<EntityInfo, String> {
    let g = graph(vault);
    let es = entities(vault);
    let entity = es.iter().find(|e| e.path == path).cloned().ok_or("That note isn't part of the world.")?;
    let name_of = |p: &str| es.iter().find(|e| e.path == p).map(|e| e.name.clone()).unwrap_or_else(|| p.to_string());
    let mut relations = vec![];
    let mut backlinks = vec![];
    let mut appears_in = vec![];
    for e in &g.edges {
        match e.kind.as_str() {
            "relation" if e.from == path => relations.push((e.to.clone(), name_of(&e.to), e.label.clone(), true)),
            "relation" if e.to == path => relations.push((e.from.clone(), name_of(&e.from), e.label.clone(), false)),
            "link" if e.to == path => backlinks.push((e.from.clone(), name_of(&e.from))),
            "mention" | "scene" if e.from == path => {
                if let Some(n) = g.nodes.iter().find(|n| n.id == e.to) {
                    appears_in.push((n.manuscript.clone().unwrap_or_default(), n.block.clone().unwrap_or_default(), n.label.clone(), e.weight));
                }
            }
            _ => {}
        }
    }
    Ok(EntityInfo { entity, relations, backlinks, appears_in })
}

// ---------- timeline ----------

/// Find a world note by name, alias or path.
pub fn find(vault: &Path, name: &str) -> Option<Entity> {
    let es = entities(vault);
    let idx = Index::new(&es);
    idx.get(name).cloned()
}

/// "Manuscript Title#Block title", the form `scenes:` uses.
pub fn scene_ref(vault: &Path, ms: &str, block: &str) -> Result<String, String> {
    let m = manuscript::manifest(vault, ms)?;
    let b = m.blocks.iter().find(|b| b.id == block).ok_or("No such block")?;
    Ok(format!("{}#{}", m.title, b.title))
}

#[derive(Serialize)]
pub struct TimelineEvent {
    pub path: String,
    pub name: String,
    pub when: Option<String>,
    pub summary: String,
    pub status: Option<String>,
    /// Path of the event this one follows, if any.
    pub after: Option<String>,
    /// Which timeline it's on: "" for the main one, else the `timeline:` name
    /// set on it or on an event before it.
    pub timeline: String,
    /// 0 for a timeline's main line; 1+ for branches.
    pub lane: usize,
    /// Scenes it happens in: (manuscript id, block id, label, position in the manuscript).
    pub scenes: Vec<(String, String, String, usize)>,
    pub people: Vec<String>,
    /// Where the telling read against tells it first (a block position), or
    /// None if that telling doesn't tell it.
    pub told_at: Option<usize>,
    /// Out of order on its line, told before events that happen earlier.
    pub told_earlier: bool,
    /// Out of order on its line, told after events that happen later: a flashback.
    pub told_later: bool,
}

/// Mark the events told out of order on one timeline's main line: those
/// outside the longest run told in order. Each is "earlier" if told before
/// the in-order event above it, else "later". On a tie the run keeps the
/// events that happen first, so a jump ahead reads as told earlier.
fn flag_order(events: &mut [TimelineEvent]) {
    let line: Vec<usize> = (0..events.len()).filter(|&i| events[i].lane == 0 && events[i].told_at.is_some()).collect();
    // Longest non-decreasing run, found backwards so ties keep earlier events.
    let r: Vec<i64> = line.iter().rev().map(|&i| -(events[i].told_at.unwrap() as i64)).collect();
    let (mut tails, mut tail_at, mut prev): (Vec<i64>, Vec<usize>, Vec<Option<usize>>) = (vec![], vec![], vec![None; r.len()]);
    for (k, &x) in r.iter().enumerate() {
        let p = tails.partition_point(|&t| t <= x);
        prev[k] = p.checked_sub(1).map(|q| tail_at[q]);
        if p == tails.len() {
            tails.push(x);
            tail_at.push(k);
        } else {
            tails[p] = x;
            tail_at[p] = k;
        }
    }
    let mut keep = vec![false; r.len()];
    let mut at = tail_at.last().copied();
    while let Some(k) = at {
        keep[k] = true;
        at = prev[k];
    }
    let in_order: HashSet<usize> = (0..r.len()).filter(|&k| keep[k]).map(|k| line[line.len() - 1 - k]).collect();
    let mut above: Option<usize> = None;
    for &i in &line {
        let told = events[i].told_at.unwrap();
        if in_order.contains(&i) {
            above = Some(told);
        } else if above.is_some_and(|a| told < a) {
            events[i].told_earlier = true;
        } else {
            events[i].told_later = true;
        }
    }
}

/// The `timeline:` an event sets for itself, if any ("" says the main one).
fn own_timeline(e: &Entity) -> Option<String> {
    e.props["timeline"].as_str().map(|t| t.trim().to_string())
}

/// Which timeline an event is on: its own `timeline:`, else its
/// predecessor's; "" is the main one.
fn timeline_of(e: &Entity, idx: &Index) -> String {
    let mut cur = e;
    let mut seen = HashSet::new();
    loop {
        if let Some(t) = own_timeline(cur) {
            return t;
        }
        if !seen.insert(cur.path.clone()) {
            return String::new();
        }
        match cur.after.iter().find_map(|t| idx.get(t).filter(|p| p.kind == "event")) {
            Some(p) => cur = p,
            None => return String::new(),
        }
    }
}

/// Block id -> (position, label of the take it uses).
pub type Telling = HashMap<String, (usize, String)>;

/// Block id -> (position, label of the take it uses), in a manuscript's
/// current cut (order, live takes, pages left out skipped), or in one of its cuts with that cut's
/// picks. A cut tells only the blocks in its order.
pub fn telling(vault: &Path, ms: &str, cut: Option<&str>) -> Result<(String, Telling), String> {
    let want = ms.trim().to_lowercase();
    let info = manuscript::list(vault).into_iter().find(|m| m.id == want || m.title.to_lowercase() == want).ok_or_else(|| format!("No manuscript {ms:?}"))?;
    let m = manuscript::manifest(vault, &info.id)?;
    let cut = cut.map(|c| m.cuts.iter().find(|x| x.name == c).ok_or_else(|| format!("No cut {c:?} in {}", m.title))).transpose()?;
    let order: Vec<&str> = match cut {
        None => m.blocks.iter().filter(|b| !b.out).map(|b| b.id.as_str()).collect(),
        Some(c) => c.order.iter().map(String::as_str).collect(),
    };
    let mut out = HashMap::new();
    for id in order {
        let Some(b) = m.blocks.iter().find(|b| b.id == id) else { continue };
        let take = cut.and_then(|c| c.picks.get(id)).filter(|t| b.variants.iter().any(|v| &v.id == *t)).unwrap_or(&b.live);
        let label = b.variants.iter().find(|v| &v.id == take).map(|v| v.label.clone()).unwrap_or_default();
        let n = out.len();
        out.insert(b.id.clone(), (n, label));
    }
    Ok((info.id, out))
}

/// Events in story order, read against the manuscript's current order.
pub fn timeline(vault: &Path) -> Vec<TimelineEvent> {
    timeline_read(vault, None).unwrap_or_default()
}

/// Events in story order, one timeline after another (the main one first).
/// Follow each chain from its start; when an event has several successors,
/// the first continues the lane and the rest branch off. An event that sets
/// a different `timeline:` from the one it follows starts that timeline,
/// branching from there.
///
/// `against` is (manuscript, cut) to measure "told earlier" by: positions in
/// that manuscript, or in that cut of it. Without it, every scene counts at
/// its position in its own manuscript.
pub fn timeline_read(vault: &Path, against: Option<(&str, Option<&str>)>) -> Result<Vec<TimelineEvent>, String> {
    let es = entities(vault);
    let idx = Index::new(&es);
    let scenes = scenes(vault);
    let telling = against.map(|(ms, cut)| telling(vault, ms, cut)).transpose()?;
    let events: Vec<&Entity> = es.iter().filter(|e| e.kind == "event").collect();
    let prev_of = |e: &Entity| e.after.iter().find_map(|t| idx.get(t).filter(|p| p.kind == "event")).map(|p| p.path.clone());
    let mut children: HashMap<String, Vec<&Entity>> = HashMap::new();
    let mut roots: Vec<&Entity> = vec![];
    for e in &events {
        match prev_of(e) {
            Some(p) => children.entry(p).or_default().push(e),
            None => roots.push(e),
        }
    }
    // How long the chain from each event runs; the longest continuation is the main line.
    fn depth(path: &str, children: &HashMap<String, Vec<&Entity>>, memo: &mut HashMap<String, usize>, guard: &mut HashSet<String>) -> usize {
        if let Some(&d) = memo.get(path) {
            return d;
        }
        if !guard.insert(path.to_string()) {
            return 0; // cycle
        }
        let d = 1 + children.get(path).map_or(0, |ks| ks.iter().map(|k| depth(&k.path, children, memo, guard)).max().unwrap_or(0));
        memo.insert(path.to_string(), d);
        d
    }
    let mut memo = HashMap::new();
    let keys: Vec<String> = children.keys().cloned().collect();
    for k in &keys {
        depth(k, &children, &mut memo, &mut HashSet::new());
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| {
            let (da, db) = (memo.get(&a.path).copied().unwrap_or(1), memo.get(&b.path).copied().unwrap_or(1));
            db.cmp(&da).then(a.name.cmp(&b.name))
        });
    }
    roots.sort_by(|a, b| a.when.cmp(&b.when).then(a.name.cmp(&b.name)));

    // Timelines in the order they start: the main one, then named ones by
    // their first root, then any begun by an event inside another.
    let mut order: Vec<String> = vec![String::new()];
    let mut starts: HashMap<String, Vec<&Entity>> = HashMap::new();
    for r in &roots {
        let t = own_timeline(r).unwrap_or_default();
        if !order.contains(&t) {
            order.push(t.clone());
        }
        starts.entry(t).or_default().push(r);
    }

    // `takes:` on an event names the takes that tell it (by label), for a
    // block whose versions don't all include it; a telling counts the event
    // only where it uses one of those takes.
    let told_at = |e: &Entity| -> Option<usize> {
        let found = e.scenes.iter().filter_map(|s| find_scene(&scenes, s));
        let takes: Vec<String> = strings(e.props.get("takes")).iter().map(|t| t.to_lowercase()).collect();
        match &telling {
            Some((ms, pos)) => found
                .filter(|s| &s.ms == ms)
                .filter_map(|s| pos.get(&s.block))
                .filter(|(_, label)| takes.is_empty() || takes.contains(&label.to_lowercase()))
                .map(|(p, _)| *p)
                .min(),
            None => found.map(|s| s.index).min(),
        }
    };

    let mut out = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut lanes = 0usize;
    let mut i = 0;
    while i < order.len() {
        let tl = order[i].clone();
        i += 1;
        let first = out.len();
        // Iterative DFS so long chains can't overflow the stack.
        let mut stack: Vec<(&Entity, usize)> = starts.remove(&tl).unwrap_or_default().into_iter().rev().map(|r| (r, 0)).collect();
        while let Some((e, lane)) = stack.pop() {
            if !seen.insert(e.path.clone()) {
                continue;
            }
            out.push(TimelineEvent {
                path: e.path.clone(),
                name: e.name.clone(),
                when: e.when.clone(),
                summary: e.summary.clone(),
                status: e.status.clone(),
                after: prev_of(e),
                timeline: tl.clone(),
                lane,
                scenes: e
                    .scenes
                    .iter()
                    .filter_map(|s| find_scene(&scenes, s))
                    .map(|s| (s.ms.clone(), s.block.clone(), s.label.clone(), s.index))
                    .collect(),
                people: e.links.iter().filter_map(|l| idx.get(l)).filter(|t| t.kind == "character").map(|t| t.name.clone()).collect(),
                told_at: told_at(e),
                told_earlier: false,
                told_later: false,
            });
            if let Some(kids) = children.get(&e.path) {
                // A successor on another timeline starts that one (later).
                let (same, other): (Vec<&Entity>, Vec<&Entity>) = kids.iter().partition(|k| own_timeline(k).is_none_or(|t| t == tl));
                for k in other {
                    let t = own_timeline(k).unwrap_or_default();
                    if !order.contains(&t) {
                        order.push(t.clone());
                    } else if order[..i].contains(&t) {
                        // That timeline is already laid out: put this start in its own pass.
                        order.push(t.clone());
                    }
                    starts.entry(t).or_default().push(k);
                }
                // Push branches first so the main continuation is visited next.
                for (j, k) in same.iter().enumerate().rev() {
                    let l = if j == 0 { lane } else {
                        lanes += 1;
                        lanes
                    };
                    stack.push((k, l));
                }
            }
        }
        flag_order(&mut out[first..]);
    }
    // Events caught in a cycle are never reached from a root; list them last.
    for e in events.iter().filter(|e| !seen.contains(&e.path)) {
        out.push(TimelineEvent {
            path: e.path.clone(),
            name: e.name.clone(),
            when: e.when.clone(),
            summary: e.summary.clone(),
            status: Some("cycle".into()),
            after: prev_of(e),
            timeline: timeline_of(e, &idx),
            lane: 0,
            scenes: vec![],
            people: vec![],
            told_at: None,
            told_earlier: false,
            told_later: false,
        });
    }
    Ok(out)
}

/// Set (or remove, with None) one top-level frontmatter property, editing
/// only that line so the rest of the note stays exactly as written.
pub fn set_prop(text: &str, key: &str, value: Option<&str>) -> String {
    let line = value.map(|v| format!("{key}: {v}"));
    let (fm, body) = split_frontmatter(text);
    let Some(fm) = fm else {
        return match line {
            Some(l) => format!("---\n{l}\n---\n\n{text}"),
            None => text.to_string(),
        };
    };
    let mut lines: Vec<String> = vec![];
    let mut replaced = false;
    let mut skipping = false;
    for l in fm.lines() {
        let is_key = l.starts_with(&format!("{key}:"));
        if skipping && (l.starts_with(' ') || l.starts_with('-')) {
            continue; // the old value's continuation lines
        }
        skipping = false;
        if is_key {
            skipping = true;
            if let Some(nl) = &line {
                if !replaced {
                    lines.push(nl.clone());
                }
            }
            replaced = true;
            continue;
        }
        lines.push(l.to_string());
    }
    if !replaced {
        if let Some(nl) = line {
            lines.push(nl);
        }
    }
    format!("---\n{}\n---\n\n{}", lines.join("\n"), body)
}

fn quote_link(name: &str) -> String {
    format!("\"[[{name}]]\"")
}

fn event_entities(vault: &Path) -> Vec<Entity> {
    entities(vault).into_iter().filter(|e| e.kind == "event").collect()
}

/// Rewrite events so that each one in `paths` follows `after`.
fn repoint(vault: &Path, paths: &[String], after: Option<&str>) -> Result<(), String> {
    for p in paths {
        let text = crate::vault::read(vault, p)?;
        crate::vault::write(vault, p, &set_prop(&text, "after", after.map(quote_link).as_deref()))?;
    }
    Ok(())
}

fn successors(es: &[Entity], of: &str) -> Vec<String> {
    let idx = Index::new(es);
    es.iter()
        .filter(|e| e.after.iter().any(|t| idx.get(t).is_some_and(|p| p.path == of)))
        .map(|e| e.path.clone())
        .collect()
}

fn name_of(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Create a note of `kind` named `name` (refusing to overwrite), with extra
/// properties and a body. Returns its path.
pub fn add(vault: &Path, kind: &str, name: &str, props: &[(String, String)], body: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\', ':']) {
        return Err(format!("Not a usable note name: {name:?}"));
    }
    let folder = TYPES.iter().find(|(k, _)| *k == kind).map(|(_, f)| *f).unwrap_or("things");
    let rel = if folder.is_empty() { format!("{FOLDER}/{name}.md") } else { format!("{FOLDER}/{folder}/{name}.md") };
    if crate::vault::resolve(vault, &rel)?.exists() {
        return Err(format!("{rel} already exists."));
    }
    let mut fm = format!("type: {kind}\n");
    for (k, v) in props {
        fm.push_str(&format!("{k}: {v}\n"));
    }
    let body = if body.trim().is_empty() { format!("# {name}\n") } else { body.trim().to_string() + "\n" };
    crate::vault::write(vault, &rel, &format!("---\n{fm}---\n\n{body}"))?;
    Ok(rel)
}

/// Write (or clear) an event's own `timeline:`.
fn set_timeline(vault: &Path, path: &str, timeline: Option<&str>) -> Result<(), String> {
    let text = crate::vault::read(vault, path)?;
    let value = timeline.map(|t| serde_json::to_string(t).unwrap());
    let next = set_prop(&text, "timeline", value.as_deref());
    if next != text {
        crate::vault::write(vault, path, &next)?;
    }
    Ok(())
}

/// Events about to start a chain (nothing before them any more) keep the
/// timeline they were on by naming it themselves.
fn keep_timelines(vault: &Path, es: &[Entity], idx: &Index, paths: &[String]) -> Result<(), String> {
    for p in paths {
        if let Some(e) = es.iter().find(|e| &e.path == p) {
            let t = timeline_of(e, idx);
            if own_timeline(e).is_none() && !t.is_empty() {
                set_timeline(vault, p, Some(&t))?;
            }
        }
    }
    Ok(())
}

/// Add a timeline event after `after` (or at the start). With `before`, the
/// new event is spliced in between: `before` is re-pointed to follow it.
/// A new first event takes over the timeline of the one it goes before,
/// unless `props` names a timeline.
pub fn insert_event(
    vault: &Path,
    name: &str,
    after: Option<&str>,
    before: Option<&str>,
    mut props: Vec<(String, String)>,
    body: &str,
) -> Result<String, String> {
    let _lock = manuscript::Lock::take(&vault.join(".writepad").join(".timeline.lock"))?;
    let es = event_entities(vault);
    let idx = Index::new(&es);
    let tl_at = |p: &str| es.iter().find(|e| e.path == p).map(|e| timeline_of(e, &idx)).unwrap_or_default();
    let given = props.iter().find(|(k, _)| k == "timeline").and_then(|(_, v)| serde_json::from_str::<String>(v).ok());
    let mine = match (&given, after, before) {
        (Some(t), _, _) => t.clone(),
        (None, Some(a), _) => tl_at(a),
        (None, None, Some(b)) => tl_at(b),
        (None, None, None) => String::new(),
    };
    if let Some(a) = after {
        props.insert(0, ("after".into(), quote_link(&name_of(a))));
    } else if given.is_none() && !mine.is_empty() {
        props.push(("timeline".into(), serde_json::to_string(&mine).unwrap()));
    }
    let rel = add(vault, "event", name, &props, body)?;
    if let Some(b) = before {
        let was = tl_at(b);
        repoint(vault, &[b.to_string()], Some(name))?;
        // It now follows the new event: inherit, unless it was on another timeline.
        set_timeline(vault, b, if was == mine { None } else { Some(&was) })?;
    }
    Ok(rel)
}

/// Move an event to follow `after` (None = the start): its old successors
/// close the gap, and with `before` it's spliced in ahead of that event.
/// Dropped after an event, it joins that event's timeline; moved to the
/// start ahead of `before`, it takes over that timeline.
pub fn move_event(vault: &Path, path: &str, after: Option<&str>, before: Option<&str>) -> Result<(), String> {
    let _lock = manuscript::Lock::take(&vault.join(".writepad").join(".timeline.lock"))?;
    let es = event_entities(vault);
    let me = es.iter().find(|e| e.path == path).ok_or("Not a timeline event")?;
    if after == Some(path) || before == Some(path) {
        return Ok(());
    }
    let idx = Index::new(&es);
    let old_prev = me.after.iter().find_map(|t| idx.get(t)).map(|p| p.name.clone());
    let succ = successors(&es, path);
    let before_tl = before.and_then(|b| es.iter().find(|e| e.path == b)).map(|e| timeline_of(e, &idx));
    // Close the gap where it was.
    if old_prev.is_none() {
        keep_timelines(vault, &es, &idx, &succ)?;
    }
    repoint(vault, &succ, old_prev.as_deref())?;
    // Splice it in where it goes.
    let after_name = after.map(name_of);
    repoint(vault, &[path.to_string()], after_name.as_deref())?;
    match after {
        Some(_) => set_timeline(vault, path, None)?,
        None => {
            let t = before_tl.unwrap_or_else(|| timeline_of(me, &idx));
            set_timeline(vault, path, Some(&t).filter(|t| !t.is_empty()).map(|t| t.as_str()))?
        }
    }
    if let Some(b) = before {
        repoint(vault, &[b.to_string()], Some(&me.name))?;
        if after.is_none() {
            set_timeline(vault, b, None)?;
        }
    }
    Ok(())
}

/// Remove an event from the chain (its successors follow its predecessor)
/// and move its note to the trash. Returns the trash id.
pub fn delete_event(vault: &Path, path: &str) -> Result<String, String> {
    let _lock = manuscript::Lock::take(&vault.join(".writepad").join(".timeline.lock"))?;
    let es = event_entities(vault);
    let me = es.iter().find(|e| e.path == path).ok_or("Not a timeline event")?;
    let idx = Index::new(&es);
    let prev = me.after.iter().find_map(|t| idx.get(t)).map(|p| p.name.clone());
    let succ = successors(&es, path);
    if prev.is_none() {
        keep_timelines(vault, &es, &idx, &succ)?;
    }
    repoint(vault, &succ, prev.as_deref())?;
    crate::vault::trash(vault, path, succ)
}

/// Delete any note: events leave the timeline cleanly, everything goes to
/// the trash. Returns the trash id for [`restore_note`].
pub fn delete_note(vault: &Path, path: &str) -> Result<String, String> {
    let is_event = entities(vault).iter().any(|e| e.path == path && e.kind == "event");
    if is_event { delete_event(vault, path) } else { crate::vault::trash(vault, path, vec![]) }
}

/// Undo [`delete_note`]: the note comes back, and an event is spliced back
/// in ahead of the events that followed it.
pub fn restore_note(vault: &Path, id: &str) -> Result<String, String> {
    let _lock = manuscript::Lock::take(&vault.join(".writepad").join(".timeline.lock"))?;
    let record = crate::vault::restore(vault, id)?;
    repoint(vault, &record.successors, Some(&name_of(&record.path)))?;
    Ok(record.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("writepad-world-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn summary_reads_past_headings() {
        assert_eq!(summary_of("# Testing\n"), "Testing");
        assert_eq!(summary_of("# Title\nFirst line\nsecond.\n\nMore."), "First line second.");
        assert_eq!(summary_of("\n## A\n\nBody here."), "Body here.");
    }

    #[test]
    fn parses_frontmatter_links_and_relations() {
        let e = parse(
            "world/characters/June.md",
            "---\ntype: character\naliases: [Junie]\nrelationships:\n  - { to: \"[[Dolores]]\", kind: learns from }\n  - \"[[Robin]]\"\n---\n\n# June\n\nCounts the figures with [[Robin]] in [[Unit 12|the ward]].\n",
        );
        assert_eq!(e.kind, "character");
        assert_eq!(e.aliases, ["Junie"]);
        assert_eq!(e.relations.len(), 2);
        assert_eq!(e.relations[0].kind, "learns from");
        assert!(e.links.contains(&"Unit 12".to_string()));
        assert_eq!(e.summary, "Counts the figures with Robin in the ward.");
    }

    #[test]
    fn set_prop_edits_one_line() {
        let t = "---\ntype: event\nafter: \"[[A]]\"\nwhen: day 1\n---\n\nBody [[x]].\n";
        assert_eq!(set_prop(t, "after", Some("\"[[B]]\"")), "---\ntype: event\nafter: \"[[B]]\"\nwhen: day 1\n---\n\nBody [[x]].\n");
        assert_eq!(set_prop(t, "after", None), "---\ntype: event\nwhen: day 1\n---\n\nBody [[x]].\n");
        assert!(set_prop("No frontmatter.", "after", Some("x")).starts_with("---\nafter: x\n---"));
    }

    #[test]
    fn timeline_is_a_linked_list_with_moves_and_branches() {
        let v = vault("timeline");
        let a = insert_event(&v, "A", None, None, vec![], "").unwrap();
        let b = insert_event(&v, "B", Some(&a), None, vec![], "").unwrap();
        let d = insert_event(&v, "D", Some(&b), None, vec![], "").unwrap();
        // Splice C between B and D.
        let c = insert_event(&v, "C", Some(&b), Some(&d), vec![], "").unwrap();
        let names = |v: &Path| timeline(v).iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&v), ["A", "B", "C", "D"]);
        // Move D to the start.
        move_event(&v, &d, None, Some(&a)).unwrap();
        assert_eq!(names(&v), ["D", "A", "B", "C"]);
        // A branch: E also follows A.
        insert_event(&v, "E", Some(&a), None, vec![], "").unwrap();
        let t = timeline(&v);
        assert_eq!(t.iter().map(|e| (e.name.as_str(), e.lane)).collect::<Vec<_>>(), [("D", 0), ("A", 0), ("B", 0), ("C", 0), ("E", 1)]);
        // Deleting B closes the gap; restoring splices it back.
        let trash = delete_note(&v, &b).unwrap();
        assert_eq!(timeline(&v).iter().find(|e| e.name == "C").unwrap().after.as_deref(), Some(a.as_str()));
        restore_note(&v, &trash).unwrap();
        assert_eq!(timeline(&v).iter().find(|e| e.name == "C").unwrap().after.as_deref(), Some(b.as_str()));
        let _ = c;
        fs::remove_dir_all(v).unwrap();
    }

    #[test]
    fn named_timelines_keep_their_events() {
        let v = vault("named-timelines");
        let tl = |v: &Path| timeline(v).iter().map(|e| (e.name.clone(), e.timeline.clone())).collect::<Vec<_>>();
        let pair = |n: &str, t: &str| (n.to_string(), t.to_string());
        let a = insert_event(&v, "A", None, None, vec![], "").unwrap();
        insert_event(&v, "B", Some(&a), None, vec![], "").unwrap();
        let x = insert_event(&v, "X", None, None, vec![("timeline".into(), "\"1999\"".into())], "").unwrap();
        let y = insert_event(&v, "Y", Some(&x), None, vec![], "").unwrap();
        // Main first, then 1999; Y inherits from X.
        assert_eq!(tl(&v), [pair("A", ""), pair("B", ""), pair("X", "1999"), pair("Y", "1999")]);
        // A new first event ahead of X takes over the 1999 timeline.
        let w = insert_event(&v, "W", None, Some(&x), vec![], "").unwrap();
        assert_eq!(tl(&v), [pair("A", ""), pair("B", ""), pair("W", "1999"), pair("X", "1999"), pair("Y", "1999")]);
        // Deleting the first event leaves the rest on 1999.
        delete_note(&v, &w).unwrap();
        assert_eq!(tl(&v)[2..], [pair("X", "1999"), pair("Y", "1999")]);
        // Dragged after B, Y joins the main timeline.
        move_event(&v, &y, Some(&v.join("world/timeline/B.md").strip_prefix(&v).unwrap().to_string_lossy()), None).unwrap();
        assert_eq!(tl(&v), [pair("A", ""), pair("B", ""), pair("Y", ""), pair("X", "1999")]);
        // An event that names another timeline starts it, branching from where it follows.
        insert_event(&v, "Z", Some(&a), None, vec![("timeline".into(), "\"Mirror\"".into())], "").unwrap();
        let t = timeline(&v);
        let z = t.iter().find(|e| e.name == "Z").unwrap();
        assert_eq!((z.timeline.as_str(), z.lane, z.after.as_deref()), ("Mirror", 0, Some(a.as_str())));
        assert_eq!(t.last().unwrap().name, "Z", "Mirror starts after the timelines before it");
        fs::remove_dir_all(v).unwrap();
    }

    #[test]
    fn told_earlier_against_a_cut() {
        let v = vault("telling");
        let src = v.join("draft.md");
        fs::write(&src, "## One\n\nFirst.\n\n## Two\n\nSecond.\n\n## Three\n\nThird.\n").unwrap();
        let m = manuscript::import(&v, "Book", &[src.to_string_lossy().into()]).unwrap();
        let ev = |name: &str, after: Option<&str>, block: &str| {
            insert_event(&v, name, after, None, vec![("scenes".into(), format!("[\"[[Book#{block}]]\"]"))], "").unwrap()
        };
        let a = ev("A", None, "One");
        let b = ev("B", Some(&a), "Two");
        ev("C", Some(&b), "Three");
        let flags = |t: Vec<TimelineEvent>| t.iter().map(|e| (e.name.clone(), e.told_at, e.told_earlier, e.told_later)).collect::<Vec<_>>();
        let f = |n: &str, at: Option<usize>, early: bool, late: bool| (n.to_string(), at, early, late);
        assert_eq!(flags(timeline_read(&v, Some(("Book", None))).unwrap()), [f("A", Some(0), false, false), f("B", Some(1), false, false), f("C", Some(2), false, false)]);
        // A cut that tells Three first, then One, and leaves Two out.
        let id = |t: &str| m.blocks.iter().find(|b| b.title == t).unwrap().id.clone();
        manuscript::update(&v, &m.id, |man| {
            man.cuts.push(manuscript::Cut { name: "Flashback".into(), order: vec![id("Three"), id("One")], picks: Default::default() });
            Ok(())
        })
        .unwrap();
        assert_eq!(
            flags(timeline_read(&v, Some(("book", Some("Flashback")))).unwrap()),
            [f("A", Some(1), false, false), f("B", None, false, false), f("C", Some(0), true, false)]
        );
        // Told Two, Three, then One: only One is out of order, a flashback.
        manuscript::update(&v, &m.id, |man| {
            man.cuts.push(manuscript::Cut { name: "In medias res".into(), order: vec![id("Two"), id("Three"), id("One")], picks: Default::default() });
            Ok(())
        })
        .unwrap();
        assert_eq!(
            flags(timeline_read(&v, Some(("Book", Some("In medias res")))).unwrap()),
            [f("A", Some(2), false, true), f("B", Some(0), false, false), f("C", Some(1), false, false)]
        );
        assert!(timeline_read(&v, Some(("Book", Some("Nope")))).is_err());
        // A second version of the book: B only happens in the first one's take of Two.
        let src2 = v.join("revised.md");
        fs::write(&src2, "## One\n\nFirst, again.\n\n## Two\n\nSecond, changed.\n\n## Three\n\nThird, again.\n").unwrap();
        let m2 = manuscript::import(&v, "Versions", &[src.to_string_lossy().into(), src2.to_string_lossy().into()]).unwrap();
        let b2 = insert_event(&v, "B2", None, None, vec![("scenes".into(), "[\"[[Versions#Two]]\"]".into()), ("takes".into(), "[\"draft\"]".into())], "").unwrap();
        let told = |v: &Path| timeline_read(v, Some(("Versions", None))).unwrap().into_iter().find(|e| e.path == b2).unwrap().told_at;
        assert_eq!(told(&v), None, "the revised take is live, and it doesn't tell B2");
        let two = m2.blocks.iter().find(|b| b.title == "Two").unwrap();
        manuscript::set_live(&v, &m2.id, &two.variants[0].id).unwrap();
        assert_eq!(told(&v), Some(1));
        fs::remove_dir_all(v).unwrap();
    }

    #[test]
    fn month_names_are_not_people_in_dates() {
        assert_eq!(count_mentions("June counts. On June 13 and June 15th, June sat.", "June"), 2);
        assert_eq!(count_mentions("April 2 came. April laughed.", "April"), 1);
        assert_eq!(count_mentions("Robin 2", "Robin"), 1, "only month names get the date rule");
    }

    #[test]
    fn graph_finds_mentions_in_manuscripts() {
        let v = vault("graph");
        let src = v.join("draft.md");
        fs::write(&src, "## 1\n\nJune counts. Junie counts again. Dolores watches.\n\n## 2\n\nNobody here.\n").unwrap();
        manuscript::import(&v, "Book", &[src.to_string_lossy().into()]).unwrap();
        add(&v, "character", "June", &[("aliases".into(), "[Junie]".into())], "Counts.").unwrap();
        add(&v, "character", "Kai", &[], "").unwrap();
        let g = graph(&v);
        let m = g.edges.iter().find(|e| e.kind == "mention").unwrap();
        assert_eq!((m.from.as_str(), m.weight), ("world/characters/June.md", 2));
        assert!(g.issues.iter().any(|i| i.message.contains("Kai never appears")));
        fs::remove_dir_all(v).unwrap();
    }
}

/// `VAULT=... OUT=... cargo test -p writepad-core dump_world -- --ignored`
#[cfg(test)]
#[test]
#[ignore]
fn dump_world() {
    let v = std::path::PathBuf::from(std::env::var("VAULT").unwrap());
    let out = std::path::PathBuf::from(std::env::var("OUT").unwrap());
    fs::write(out.join("graph.json"), serde_json::to_string(&graph(&v)).unwrap()).unwrap();
    fs::write(out.join("timeline.json"), serde_json::to_string(&timeline(&v)).unwrap()).unwrap();
}
