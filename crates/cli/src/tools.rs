//! The tools agents get, shared by the MCP server and `writepad call`.
//!
//! Ground rules baked in here: an agent writes only to takes it created
//! (unless `force`), never silently replaces a note, and every take it makes
//! carries its name, so the person can see who wrote what.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use writepad_core::{diff, export, history, manuscript as ms, profile, script, vault, world};

pub struct Ctx {
    pub vault: PathBuf,
    /// Default author for takes, when a call doesn't name one.
    pub agent: String,
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    pub run: fn(&Ctx, &Value) -> Result<Value, String>,
}

fn s<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("Missing \"{key}\""))
}

fn opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).filter(|v| !v.is_empty())
}

fn agent<'a>(ctx: &'a Ctx, args: &'a Value) -> &'a str {
    opt(args, "agent").unwrap_or(&ctx.agent)
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Resolve a take: an explicit take id, or a block's live take.
fn resolve_take(m: &ms::Manifest, block: Option<&str>, take: Option<&str>) -> Result<(String, String), String> {
    if let Some(t) = take {
        let b = ms::block_of(m, t).ok_or_else(|| format!("No take {t:?}. Use outline to see take ids."))?;
        return Ok((b.id.clone(), t.to_string()));
    }
    let block = block.ok_or("Give a block or a take.")?;
    let b = m
        .blocks
        .iter()
        .find(|b| b.id == block || b.title == block)
        .ok_or_else(|| format!("No block {block:?}. Use outline to see block ids and titles."))?;
    Ok((b.id.clone(), b.live.clone()))
}

/// Refuse to write someone else's take unless forced.
fn check_owner(m: &ms::Manifest, take: &str, who: &str, force: bool) -> Result<(), String> {
    let v = m.blocks.iter().flat_map(|b| &b.variants).find(|v| v.id == take).ok_or_else(|| format!("No take {take:?}"))?;
    if v.author != who && !force {
        let owner = if v.author.is_empty() { "the author (a person)".to_string() } else { format!("agent {:?}", v.author) };
        return Err(format!(
            "Take {take:?} (\"{}\") belongs to {owner}. Fork it to make your own take, or pass force: true if you were asked to edit it directly.",
            v.label
        ));
    }
    Ok(())
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

fn str_prop(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

const MS: &str = "Manuscript id (from list_manuscripts).";
const AGENT: &str = "Your name, so the author can see which agent wrote what. Use the same name for every call.";

pub fn all() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_manuscripts",
            description: "List the manuscripts in the vault, with ids, block counts and take counts.",
            schema: || obj(json!({}), &[]),
            run: |ctx, _| Ok(json!(ms::list(&ctx.vault))),
        },
        Tool {
            name: "outline",
            description: "The manuscript's blocks (chapters/sections) in order. For each block: id, title, the live take (the one in the current cut), and every take with its id, label, author (\"\" = the person) and word count.",
            schema: || obj(json!({ "manuscript": str_prop(MS) }), &["manuscript"]),
            run: |ctx, a| {
                let id = s(a, "manuscript")?;
                let m = ms::load(&ctx.vault, id)?;
                let blocks: Vec<Value> = m
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        json!({
                            "index": i + 1,
                            "block": b.id,
                            "title": b.title,
                            "live": b.live,
                            "takes": b.variants.iter().map(|v| json!({
                                "take": v.id, "label": v.label, "author": v.author, "words": words(&v.text),
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                Ok(json!({ "manuscript": m.id, "title": m.title, "blocks": blocks, "cuts": m.cuts.iter().map(|c| &c.name).collect::<Vec<_>>() }))
            },
        },
        Tool {
            name: "read",
            description: "Read one take's text. Give a take id, or a block (id or exact title) to read its live take.",
            schema: || obj(json!({ "manuscript": str_prop(MS), "block": str_prop("Block id or title."), "take": str_prop("Take id.") }), &["manuscript"]),
            run: |ctx, a| {
                let id = s(a, "manuscript")?;
                let m = ms::manifest(&ctx.vault, id)?;
                let (block, take) = resolve_take(&m, opt(a, "block"), opt(a, "take"))?;
                let b = m.blocks.iter().find(|b| b.id == block).unwrap();
                let v = b.variants.iter().find(|v| v.id == take).unwrap();
                Ok(json!({
                    "block": block, "title": b.title, "take": take, "label": v.label, "author": v.author,
                    "live": b.live == take, "text": ms::read_take(&ctx.vault, id, &take)?,
                }))
            },
        },
        Tool {
            name: "fork",
            description: "Make your own take of a block to work in. Starts as a copy of `from` (default: the block's live take) unless you pass `text`. Returns the new take id; edit it with `edit` or `write`. This never changes what's live.",
            schema: || {
                obj(
                    json!({
                        "manuscript": str_prop(MS), "block": str_prop("Block id or title."),
                        "from": str_prop("Take id to copy (default: the live take)."),
                        "label": str_prop("Short name for your take, e.g. \"tighter dialogue\"."),
                        "text": str_prop("Full text for the take, instead of a copy."),
                        "agent": str_prop(AGENT),
                    }),
                    &["manuscript", "block"],
                )
            },
            run: |ctx, a| {
                let id = s(a, "manuscript")?;
                let who = agent(ctx, a);
                let m = ms::manifest(&ctx.vault, id)?;
                let (block, source) = resolve_take(&m, opt(a, "block"), opt(a, "from"))?;
                let text = match opt(a, "text") {
                    Some(t) => t.to_string(),
                    None => ms::read_take(&ctx.vault, id, &source)?,
                };
                let label = opt(a, "label").map(String::from).unwrap_or_else(|| format!("{who} take"));
                let v = ms::add_take(&ctx.vault, id, &block, Some(&source), &label, who, &text)?;
                Ok(json!({ "take": v.id, "label": v.label, "block": block, "copied_from": source, "words": words(&text) }))
            },
        },
        Tool {
            name: "edit",
            description: "Replace an exact passage in one of your takes. `old` must appear exactly once (include enough surrounding words to make it unique). Prefer this over `write` for targeted changes.",
            schema: || {
                obj(
                    json!({
                        "manuscript": str_prop(MS), "take": str_prop("Your take's id."),
                        "old": str_prop("Exact text to replace (must occur once)."), "new": str_prop("Replacement text."),
                        "agent": str_prop(AGENT),
                        "force": { "type": "boolean", "description": "Edit a take you don't own. Only if explicitly asked." },
                    }),
                    &["manuscript", "take", "old", "new"],
                )
            },
            run: |ctx, a| {
                let (id, take, old, new) = (s(a, "manuscript")?, s(a, "take")?, s(a, "old")?, s(a, "new")?);
                let m = ms::manifest(&ctx.vault, id)?;
                check_owner(&m, take, agent(ctx, a), a["force"].as_bool().unwrap_or(false))?;
                let text = ms::read_take(&ctx.vault, id, take)?;
                match text.matches(old).count() {
                    0 => return Err("`old` wasn't found in the take. Read it again; it may have changed.".into()),
                    1 => {}
                    n => return Err(format!("`old` appears {n} times. Include more surrounding text so it's unique.")),
                }
                let updated = text.replacen(old, new, 1);
                ms::write_take(&ctx.vault, id, take, &updated)?;
                Ok(json!({ "take": take, "words": words(&updated) }))
            },
        },
        Tool {
            name: "write",
            description: "Replace the whole text of one of your takes.",
            schema: || {
                obj(
                    json!({
                        "manuscript": str_prop(MS), "take": str_prop("Your take's id."), "text": str_prop("The full new text."),
                        "agent": str_prop(AGENT),
                        "force": { "type": "boolean", "description": "Write a take you don't own. Only if explicitly asked." },
                    }),
                    &["manuscript", "take", "text"],
                )
            },
            run: |ctx, a| {
                let (id, take, text) = (s(a, "manuscript")?, s(a, "take")?, s(a, "text")?);
                let m = ms::manifest(&ctx.vault, id)?;
                check_owner(&m, take, agent(ctx, a), a["force"].as_bool().unwrap_or(false))?;
                ms::write_take(&ctx.vault, id, take, text)?;
                Ok(json!({ "take": take, "words": words(text) }))
            },
        },
        Tool {
            name: "compare",
            description: "Word-level diff between two takes: [-removed-] and {+added+} inline, plus counts. Use it to describe what you changed.",
            schema: || obj(json!({ "manuscript": str_prop(MS), "a": str_prop("Take id (before)."), "b": str_prop("Take id (after).") }), &["manuscript", "a", "b"]),
            run: |ctx, a| {
                let id = s(a, "manuscript")?;
                let (x, y) = (ms::read_take(&ctx.vault, id, s(a, "a")?)?, ms::read_take(&ctx.vault, id, s(a, "b")?)?);
                let ops = diff::words(&x, &y);
                let mut marked = String::new();
                let (mut added, mut removed) = (0, 0);
                for op in &ops {
                    match op.tag {
                        "insert" => {
                            added += words(&op.text);
                            marked.push_str(&format!("{{+{}+}}", op.text));
                        }
                        "delete" => {
                            removed += words(&op.text);
                            marked.push_str(&format!("[-{}-]", op.text));
                        }
                        _ => marked.push_str(&op.text),
                    }
                }
                Ok(json!({ "words_added": added, "words_removed": removed, "diff": marked }))
            },
        },
        Tool {
            name: "set_live",
            description: "Make a take the live one for its block (what goes into the current cut and exports). Only do this when the author asked you to choose.",
            schema: || obj(json!({ "manuscript": str_prop(MS), "take": str_prop("Take id.") }), &["manuscript", "take"]),
            run: |ctx, a| {
                ms::set_live(&ctx.vault, s(a, "manuscript")?, s(a, "take")?)?;
                Ok(json!({ "ok": true }))
            },
        },
        Tool {
            name: "add_block",
            description: "Add a new block (scene/section) with your text as its first take, after the given block (default: at the end).",
            schema: || {
                obj(
                    json!({
                        "manuscript": str_prop(MS), "title": str_prop("Block title."), "text": str_prop("Its text."),
                        "after": str_prop("Block id to insert after."), "agent": str_prop(AGENT),
                    }),
                    &["manuscript", "title", "text"],
                )
            },
            run: |ctx, a| {
                let b = ms::add_block(&ctx.vault, s(a, "manuscript")?, s(a, "title")?, opt(a, "after"), agent(ctx, a), s(a, "text")?)?;
                Ok(json!({ "block": b.id, "take": b.live }))
            },
        },
        Tool {
            name: "world",
            description: "The story's world: every location, character, relationship, event and thing (notes under world/), grouped by type with one-line summaries, plus consistency issues (dangling links, names that never appear in the manuscript).",
            schema: || obj(json!({}), &[]),
            run: |ctx, _| {
                let g = world::graph(&ctx.vault);
                let mut groups: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
                for e in world::entities(&ctx.vault) {
                    groups.entry(e.kind.clone()).or_default().push(json!({
                        "name": e.name, "aliases": e.aliases, "summary": e.summary, "status": e.status, "path": e.path,
                    }));
                }
                Ok(json!({ "types": groups, "issues": g.issues }))
            },
        },
        Tool {
            name: "world_entry",
            description: "Everything about one world entry (by name or alias): its note, relationships in both directions, notes that link to it, and every manuscript block it appears in (with mention counts).",
            schema: || obj(json!({ "name": str_prop("Name or alias, e.g. \"June\".") }), &["name"]),
            run: |ctx, a| {
                let e = world::find(&ctx.vault, s(a, "name")?).ok_or_else(|| format!("No world entry {:?}. Use `world` to list them.", s(a, "name").unwrap_or("")))?;
                let info = world::entity_info(&ctx.vault, &e.path)?;
                Ok(json!({
                    "entry": info.entity,
                    "text": vault::read(&ctx.vault, &e.path)?,
                    "relations": info.relations.iter().map(|(_, name, kind, out)| json!({ "with": name, "kind": kind, "direction": if *out { "out" } else { "in" } })).collect::<Vec<_>>(),
                    "linked_from": info.backlinks.iter().map(|(_, n)| n).collect::<Vec<_>>(),
                    "appears_in": info.appears_in.iter().map(|(m, b, label, n)| json!({ "manuscript": m, "block": b, "label": label, "mentions": n })).collect::<Vec<_>>(),
                }))
            },
        },
        Tool {
            name: "scene_cast",
            description: "Who and what appears in one manuscript block (by name/alias), and which timeline events happen in it.",
            schema: || obj(json!({ "manuscript": str_prop(MS), "block": str_prop("Block id or title.") }), &["manuscript", "block"]),
            run: |ctx, a| {
                let id = s(a, "manuscript")?;
                let m = ms::manifest(&ctx.vault, id)?;
                let (block, _) = resolve_take(&m, opt(a, "block"), None)?;
                let info = world::scene_info(&ctx.vault, id, &block)?;
                Ok(json!({
                    "present": info.present.iter().map(|(e, n)| json!({ "name": e.name, "type": e.kind, "mentions": n })).collect::<Vec<_>>(),
                    "events": info.events.iter().map(|e| &e.name).collect::<Vec<_>>(),
                }))
            },
        },
        Tool {
            name: "timeline",
            description: "The story's events in chronological order (a linked list: each event follows the one before it). lane 0 is the main line; higher lanes are branches. Each event lists the manuscript scenes it happens in, so story order vs. chapter order (flashbacks) is visible.",
            schema: || obj(json!({}), &[]),
            run: |ctx, _| Ok(json!(world::timeline(&ctx.vault))),
        },
        Tool {
            name: "profile",
            description: "Measured style numbers for the books in the Library (chapter, sentence and paragraph length distributions, dialogue share, one-sentence paragraphs, adverbs, readability, how chapters end), the weighted target blend, and the same numbers for a manuscript. Counts, not opinions: use them to ground style advice.",
            schema: || obj(json!({ "manuscript": str_prop("Also measure this manuscript (id or title).") }), &[]),
            run: |ctx, a| {
                let id = match opt(a, "manuscript") {
                    Some(m) => Some(ms::manifest(&ctx.vault, m)?.id),
                    None => None,
                };
                let mut r = json!(profile::report(&ctx.vault, id.as_deref()));
                // The per-length curves are for charts; they only bloat an agent's context.
                let strip = |v: &mut Value| {
                    if let Some(o) = v.as_object_mut() {
                        o.remove("sentence_curve");
                    }
                };
                r["books"].as_array_mut().into_iter().flatten().for_each(strip);
                strip(&mut r["target"]);
                strip(&mut r["manuscript"]);
                Ok(r)
            },
        },
        Tool {
            name: "library_add",
            description: "Measure an EPUB into the Library so its style numbers can be compared and used in the target. Only numbers are stored, never the book's text.",
            schema: || {
                obj(
                    json!({
                        "path": str_prop("Absolute path to an .epub file."),
                        "title": str_prop("Title, if the file's metadata is missing or wrong."),
                        "author": str_prop("Author, if the file's metadata is missing or wrong."),
                    }),
                    &["path"],
                )
            },
            run: |ctx, a| Ok(json!(profile::book_model(&profile::add(&ctx.vault, s(a, "path")?, opt(a, "title"), opt(a, "author"))?).summary)),
        },
        Tool {
            name: "script_add",
            description: "Measure a screenplay into the Library from a URL (e.g. an IMSDb page) or a .txt/.fountain/.pdf file: scene lengths, dialogue vs action, speech lengths, characters' share of dialogue, interiors/nights, pacing. Only numbers are stored.",
            schema: || {
                obj(
                    json!({
                        "source": str_prop("URL or absolute file path."),
                        "title": str_prop("Title, if the title page is missing or wrong."),
                        "writer": str_prop("Writer, if the title page is missing or wrong."),
                    }),
                    &["source"],
                )
            },
            run: |ctx, a| Ok(json!(script::summarize(&script::add(&ctx.vault, s(a, "source")?, opt(a, "title"), opt(a, "writer"))?))),
        },
        Tool {
            name: "scripts",
            description: "Measured numbers for every screenplay in the Library (without the per-scene pacing list).",
            schema: || obj(json!({}), &[]),
            run: |ctx, _| {
                let mut v = json!(script::report(&ctx.vault));
                for x in v.as_array_mut().into_iter().flatten() {
                    x.as_object_mut().map(|o| o.remove("pacing"));
                }
                Ok(v)
            },
        },
        Tool {
            name: "adaptation",
            description: "Line a book in the Library up against its film script: which chapter each scene draws on, chapters cut, scenes invented for the film, pages per chapter, and dialogue kept word for word.",
            schema: || obj(json!({ "book": str_prop("Book id (see profile)."), "script": str_prop("Script id (see scripts).") }), &["book", "script"]),
            run: |ctx, a| Ok(json!(script::adapt(&ctx.vault, s(a, "book")?, s(a, "script")?)?)),
        },
        Tool {
            name: "cadence",
            description: "Lint one take against the target profile: its word count vs. the target's chapter lengths, median sentence, dialogue share, and flagged stretches (runs of long sentences, outlier sentences, dense paragraphs, repeated sentence openers), each with the flagged text. Flags are prompts for the author's judgement, not errors.",
            schema: || obj(json!({ "manuscript": str_prop(MS), "block": str_prop("Block id or title."), "take": str_prop("Take id.") }), &["manuscript"]),
            run: |ctx, a| {
                let id = s(a, "manuscript")?;
                let m = ms::manifest(&ctx.vault, id)?;
                let (_, take) = resolve_take(&m, opt(a, "block"), opt(a, "take"))?;
                let text = ms::read_take(&ctx.vault, id, &take)?;
                let c = profile::cadence(&text, profile::target(&ctx.vault).as_ref());
                let u: Vec<u16> = text.encode_utf16().collect();
                let flags: Vec<Value> = c
                    .flags
                    .iter()
                    .map(|f| {
                        let t = String::from_utf16_lossy(&u[f.from..f.to]);
                        let excerpt: String = t.chars().take(160).collect();
                        json!({ "kind": f.kind, "message": f.message, "text": if excerpt.len() < t.len() { excerpt + "…" } else { excerpt } })
                    })
                    .collect();
                let mut r = json!(c);
                r["flags"] = json!(flags);
                Ok(r)
            },
        },
        Tool {
            name: "world_add",
            description: "Propose a new world entry as a draft note for the author to review (marked status: draft and drafted_by you). Never overwrites. Use [[Name]] links in the body to connect it to other entries.",
            schema: || {
                obj(
                    json!({
                        "type": { "type": "string", "enum": ["world", "location", "character", "relationship", "thing"] },
                        "name": str_prop("Note name, e.g. \"Unit 12\"."),
                        "aliases": { "type": "array", "items": { "type": "string" }, "description": "Other names it goes by in the text." },
                        "relationships": { "type": "array", "items": { "type": "object", "properties": { "to": { "type": "string" }, "kind": { "type": "string" } } }, "description": "e.g. [{\"to\": \"Dolores\", \"kind\": \"learns from\"}]" },
                        "body": str_prop("Markdown body. Facts from the manuscript, with [[links]]."),
                        "manuscripts": { "type": "array", "items": { "type": "string" }, "description": "For type world only: titles of the manuscripts this world belongs to (names are only matched in these)." },
                        "agent": str_prop(AGENT),
                    }),
                    &["type", "name", "body"],
                )
            },
            run: |ctx, a| {
                let mut props = draft_props(ctx, a);
                if let Some(ms) = a["manuscripts"].as_array().filter(|m| !m.is_empty()) {
                    props.push(("manuscripts".into(), serde_json::to_string(ms).unwrap()));
                }
                if let Some(al) = a["aliases"].as_array().filter(|al| !al.is_empty()) {
                    props.push(("aliases".into(), serde_json::to_string(al).unwrap()));
                }
                if let Some(rs) = a["relationships"].as_array().filter(|r| !r.is_empty()) {
                    let items: Vec<String> = rs
                        .iter()
                        .filter_map(|r| {
                            let to = r["to"].as_str()?;
                            let kind = r["kind"].as_str().unwrap_or("related to");
                            Some(format!("  - {{ to: {}, kind: {} }}", serde_json::to_string(&format!("[[{to}]]")).ok()?, serde_json::to_string(kind).ok()?))
                        })
                        .collect();
                    props.push(("relationships".into(), format!("\n{}", items.join("\n"))));
                }
                let path = world::add(&ctx.vault, s(a, "type")?, s(a, "name")?, &props, s(a, "body")?)?;
                Ok(json!({ "path": path }))
            },
        },
        Tool {
            name: "timeline_add",
            description: "Add a draft event to the timeline after `after` (an event name; omit for the very start). Give `before` to splice it in between two events. `scenes` are \"Manuscript title#Block title\" refs where it happens.",
            schema: || {
                obj(
                    json!({
                        "name": str_prop("Short event name, e.g. \"The van doesn't brake\"."),
                        "after": str_prop("Event it follows."), "before": str_prop("Event it comes just before (splice)."),
                        "when": str_prop("In-story time, free text, e.g. \"Fri Apr 26 2041, 8:14am\"."),
                        "scenes": { "type": "array", "items": { "type": "string" } },
                        "body": str_prop("What happens, with [[links]] to who and where."),
                        "agent": str_prop(AGENT),
                    }),
                    &["name", "body"],
                )
            },
            run: |ctx, a| {
                let path_of = |key: &str| -> Result<Option<String>, String> {
                    match opt(a, key) {
                        None => Ok(None),
                        Some(n) => world::find(&ctx.vault, n).filter(|e| e.kind == "event").map(|e| Some(e.path)).ok_or_else(|| format!("No event {n:?} on the timeline.")),
                    }
                };
                let mut props = draft_props(ctx, a);
                if let Some(w) = opt(a, "when") {
                    props.push(("when".into(), serde_json::to_string(w).unwrap()));
                }
                if let Some(sc) = a["scenes"].as_array().filter(|x| !x.is_empty()) {
                    let links: Vec<String> = sc.iter().filter_map(|x| x.as_str()).map(|x| format!("[[{}]]", x.trim_start_matches("[[").trim_end_matches("]]"))).collect();
                    props.push(("scenes".into(), serde_json::to_string(&links).unwrap()));
                }
                let path = world::insert_event(&ctx.vault, s(a, "name")?, path_of("after")?.as_deref(), path_of("before")?.as_deref(), props, s(a, "body")?)?;
                Ok(json!({ "path": path }))
            },
        },
        Tool {
            name: "list_notes",
            description: "List the vault's markdown notes (paths relative to the vault).",
            schema: || obj(json!({}), &[]),
            run: |ctx, _| Ok(json!(vault::list_notes(&ctx.vault).into_iter().map(|n| n.path).collect::<Vec<_>>())),
        },
        Tool {
            name: "read_note",
            description: "Read a note by its vault-relative path.",
            schema: || obj(json!({ "path": str_prop("e.g. \"ideas/Unit 12.md\"") }), &["path"]),
            run: |ctx, a| Ok(json!({ "path": s(a, "path")?, "text": vault::read(&ctx.vault, s(a, "path")?)? })),
        },
        Tool {
            name: "write_note",
            description: "Create a new note. Refuses to replace an existing note unless overwrite is true; to add to a shared note, use append_note.",
            schema: || {
                obj(
                    json!({ "path": str_prop("Vault-relative path ending in .md."), "text": str_prop("Note text."),
                            "overwrite": { "type": "boolean", "description": "Replace an existing note. Only if explicitly asked." } }),
                    &["path", "text"],
                )
            },
            run: |ctx, a| {
                let path = s(a, "path")?;
                if !path.ends_with(".md") {
                    return Err("Note paths end in .md".into());
                }
                if vault::resolve(&ctx.vault, path)?.exists() && !a["overwrite"].as_bool().unwrap_or(false) {
                    return Err(format!("{path} already exists. Use append_note, or a new path."));
                }
                vault::write(&ctx.vault, path, s(a, "text")?)?;
                Ok(json!({ "path": path }))
            },
        },
        Tool {
            name: "append_note",
            description: "Append a paragraph to a note (creating it if needed). Safe when several agents add to the same note at once.",
            schema: || obj(json!({ "path": str_prop("Vault-relative path ending in .md."), "text": str_prop("Text to add."), "agent": str_prop(AGENT) }), &["path", "text"]),
            run: |ctx, a| {
                let path = s(a, "path")?;
                if !path.ends_with(".md") {
                    return Err("Note paths end in .md".into());
                }
                let who = agent(ctx, a);
                let text = if who.is_empty() { s(a, "text")?.to_string() } else { format!("**{who}:** {}", s(a, "text")?) };
                vault::append(&ctx.vault, path, &text)?;
                Ok(json!({ "path": path }))
            },
        },
        Tool {
            name: "snapshot",
            description: "Save a named version of the whole vault in its history (git). Do this after finishing a meaningful piece of work. Returns null if nothing changed.",
            schema: || obj(json!({ "message": str_prop("What changed, e.g. \"agent-2: tightened chapter 3 dialogue\"."), "agent": str_prop(AGENT) }), &["message"]),
            run: |ctx, a| {
                if !history::status(&ctx.vault).enabled {
                    return Err("History is off for this vault. The author can turn it on in WRITEpad.".into());
                }
                Ok(match history::snapshot(&ctx.vault, s(a, "message")?)? {
                    Some(snap) => json!(snap),
                    // Another writer's snapshot already picked up everything, ours included.
                    None => json!({ "snapshot": null, "note": "Nothing new since the last snapshot; your work is already saved in it." }),
                })
            },
        },
        Tool {
            name: "history",
            description: "Past versions of a take (manuscript + take) or a note (path), newest first, with a preview of each.",
            schema: || obj(json!({ "manuscript": str_prop(MS), "take": str_prop("Take id."), "path": str_prop("Note path.") }), &[]),
            run: |ctx, a| {
                let entries = match (opt(a, "path"), opt(a, "manuscript"), opt(a, "take")) {
                    (Some(p), _, _) => history::note_history(&ctx.vault, p, 50)?,
                    (None, Some(m), Some(t)) => history::take_history(&ctx.vault, m, t, 50)?,
                    _ => return Err("Give a note path, or a manuscript and take.".into()),
                };
                Ok(json!(entries
                    .iter()
                    .map(|e| json!({ "snapshot": e.snapshot.id, "message": e.snapshot.message, "time": e.snapshot.time,
                                     "words": words(&e.text), "preview": e.text.chars().take(160).collect::<String>() }))
                    .collect::<Vec<_>>()))
            },
        },
        Tool {
            name: "export",
            description: "Export the manuscript's current cut (block order, live takes) to a file: md, txt, epub or pdf.",
            schema: || {
                obj(
                    json!({ "manuscript": str_prop(MS), "format": { "type": "string", "enum": ["md", "txt", "epub", "pdf"] },
                            "path": str_prop("Output file path (absolute, or relative to the vault)."), "author": str_prop("Byline.") }),
                    &["manuscript", "format", "path"],
                )
            },
            run: |ctx, a| {
                let m = ms::load(&ctx.vault, s(a, "manuscript")?)?;
                let book = export::Book {
                    title: m.title.clone(),
                    author: opt(a, "author").unwrap_or("").to_string(),
                    chapters: ms::cut_chapters(&m).into_iter().map(|(title, text)| export::Chapter { title, text }).collect(),
                };
                let path = Path::new(s(a, "path")?);
                let path = if path.is_absolute() { path.to_path_buf() } else { ctx.vault.join(path) };
                export::write_book(&book, s(a, "format")?, &path, &std::env::temp_dir().join("writepad-export"))?;
                Ok(json!({ "path": path }))
            },
        },
        Tool {
            name: "import",
            description: "Import drafts (markdown split on ## headings, EPUB, or PDF) as a new manuscript. Same-titled sections across files become takes of one block.",
            schema: || obj(json!({ "title": str_prop("Manuscript title."), "paths": { "type": "array", "items": { "type": "string" }, "description": "Files, oldest draft first." } }), &["title", "paths"]),
            run: |ctx, a| {
                let paths: Vec<String> = a["paths"].as_array().ok_or("Missing \"paths\"")?.iter().filter_map(|p| p.as_str().map(String::from)).collect();
                let m = ms::import(&ctx.vault, s(a, "title")?, &paths)?;
                Ok(json!({ "manuscript": m.id, "blocks": m.blocks.len() }))
            },
        },
    ]
}

/// Frontmatter that marks an agent's proposal for the author to review.
fn draft_props(ctx: &Ctx, a: &Value) -> Vec<(String, String)> {
    vec![("status".into(), "draft".into()), ("drafted_by".into(), serde_json::to_string(agent(ctx, a)).unwrap())]
}

pub const INSTRUCTIONS: &str = "WRITEpad is a writing vault: manuscripts split into blocks (chapters/sections), each holding several takes (versions) of its text, plus markdown notes. Several agents and the author may be working at once.

How to work:
1. list_manuscripts, then outline to see blocks and takes. read to get text.
2. To change a block, fork it to get your own take, then use edit (targeted) or write (whole text) on that take. Pass the same `agent` name on every call.
3. You can only write takes you created. The author's takes and other agents' takes are read-only to you unless the author explicitly asked you to edit them (force: true).
4. Don't set_live unless asked to choose; the author picks which take is live by comparing takes in WRITEpad.
5. The world (locations, characters, relationships, timeline) lives in notes under world/. Read it with world, world_entry, scene_cast and timeline. Propose additions with world_add and timeline_add; they arrive as drafts for the author to accept.
6. For style questions, profile gives measured numbers for the reference books in the Library and the author's target blend; cadence lints a take against that target. Ground style suggestions in those numbers, and treat flags as places to look, not rules.
7. Use compare to summarize what you changed, append_note for shared notes and findings, and snapshot with a clear message when you finish a meaningful piece of work.";
