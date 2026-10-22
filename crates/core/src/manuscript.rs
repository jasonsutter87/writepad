//! Manuscripts: a script split into blocks, each block a stack of takes
//! (versions). Built for many writers at once — the app, plus any number of
//! agents in separate processes.
//!
//! On disk, under `<vault>/.writepad/manuscripts/<id>/`:
//!
//! ```text
//! manuscript.json        order, titles, which take is live, cuts (small)
//! variants/<take-id>.md  the text of one take
//! .lock                  held while manuscript.json is read-modify-written
//! ```
//!
//! Text writes touch only their own take's file, so writers on different
//! takes never collide. Structural changes go through [`update`], which holds
//! the lock, re-reads the manifest from disk, applies the change and writes it
//! back atomically, so concurrent changes are serialized instead of lost.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VariantMeta {
    pub id: String,
    pub label: String,
    /// Who made this take: "" for the person at the keyboard, else an agent's name.
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub created: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockMeta {
    pub id: String,
    pub title: String,
    pub variants: Vec<VariantMeta>,
    /// Id of the take that goes into the current cut.
    pub live: String,
    /// Front matter: exported as-is, without a `## title` heading.
    #[serde(default)]
    pub raw: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Cut {
    pub name: String,
    pub order: Vec<String>,
    /// block id -> take id
    pub picks: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub blocks: Vec<BlockMeta>,
    #[serde(default)]
    pub cuts: Vec<Cut>,
    /// Sections were separated by `---` rules in the source.
    #[serde(default)]
    pub separator: bool,
    /// Bumped on every structural change.
    #[serde(default)]
    pub rev: u64,
}

/// A take with its text, as handed to the app.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Variant {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub created: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Block {
    pub id: String,
    pub title: String,
    pub variants: Vec<Variant>,
    pub live: String,
    #[serde(default)]
    pub raw: bool,
}

/// A whole manuscript with every take's text.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manuscript {
    pub id: String,
    pub title: String,
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub cuts: Vec<Cut>,
    #[serde(default)]
    pub separator: bool,
    #[serde(default)]
    pub rev: u64,
}

#[derive(Serialize)]
pub struct ManuscriptInfo {
    pub id: String,
    pub title: String,
    pub blocks: usize,
    pub variants: usize,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn root(vault: &Path) -> PathBuf {
    vault.join(".writepad").join("manuscripts")
}

fn check_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("Bad id: {id:?}"));
    }
    Ok(())
}

fn ms_dir(vault: &Path, id: &str) -> Result<PathBuf, String> {
    check_id(id)?;
    Ok(root(vault).join(id))
}

fn take_path(vault: &Path, ms: &str, take: &str) -> Result<PathBuf, String> {
    check_id(take)?;
    Ok(ms_dir(vault, ms)?.join("variants").join(format!("{take}.md")))
}

/// Vault-relative path of a take's text file (for history).
pub fn take_rel_path(ms: &str, take: &str) -> String {
    format!(".writepad/manuscripts/{ms}/variants/{take}.md")
}

/// Write via a unique temp file and rename, so readers never see half a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), new_id()));
    fs::write(&tmp, data).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// An exclusive lock on a lock file, released when dropped.
pub(crate) struct Lock(File);

impl Lock {
    pub(crate) fn take(path: &Path) -> Result<Lock, String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let f = File::options().create(true).truncate(false).write(true).open(path).map_err(|e| e.to_string())?;
        f.lock().map_err(|e| format!("Couldn't lock {}: {e}", path.display()))?;
        Ok(Lock(f))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn read_manifest(vault: &Path, id: &str) -> Result<Manifest, String> {
    migrate_v1(vault, id)?;
    let path = ms_dir(vault, id)?.join("manuscript.json");
    let s = fs::read_to_string(&path).map_err(|_| format!("No manuscript {id:?}"))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_manifest(vault: &Path, m: &Manifest) -> Result<(), String> {
    let json = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    write_atomic(&ms_dir(vault, &m.id)?.join("manuscript.json"), json.as_bytes())
}

/// Apply a structural change under the manuscript's lock, against a fresh
/// read from disk, then write it back with `rev` bumped.
pub fn update<T>(vault: &Path, id: &str, f: impl FnOnce(&mut Manifest) -> Result<T, String>) -> Result<T, String> {
    let _lock = Lock::take(&ms_dir(vault, id)?.join(".lock"))?;
    let mut m = read_manifest(vault, id)?;
    let out = f(&mut m)?;
    m.rev += 1;
    write_manifest(vault, &m)?;
    Ok(out)
}

pub fn manifest(vault: &Path, id: &str) -> Result<Manifest, String> {
    read_manifest(vault, id)
}

pub fn read_take(vault: &Path, ms: &str, take: &str) -> Result<String, String> {
    match fs::read_to_string(take_path(vault, ms, take)?) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Replace a take's text. Only that take's file is touched.
pub fn write_take(vault: &Path, ms: &str, take: &str, text: &str) -> Result<(), String> {
    let m = read_manifest(vault, ms)?;
    if !m.blocks.iter().any(|b| b.variants.iter().any(|v| v.id == take)) {
        return Err(format!("No take {take:?} in {ms:?}"));
    }
    write_atomic(&take_path(vault, ms, take)?, text.as_bytes())
}

pub fn load(vault: &Path, id: &str) -> Result<Manuscript, String> {
    let m = read_manifest(vault, id)?;
    let blocks = m
        .blocks
        .into_iter()
        .map(|b| {
            let variants = b
                .variants
                .into_iter()
                .map(|v| {
                    let text = read_take(vault, id, &v.id)?;
                    Ok(Variant { id: v.id, label: v.label, author: v.author, created: v.created, text })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Block { id: b.id, title: b.title, variants, live: b.live, raw: b.raw })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Manuscript { id: m.id, title: m.title, blocks, cuts: m.cuts, separator: m.separator, rev: m.rev })
}

pub fn list(vault: &Path) -> Vec<ManuscriptInfo> {
    let Ok(entries) = fs::read_dir(root(vault)) else { return vec![] };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() {
                Some(name)
            } else {
                name.strip_suffix(".json").map(String::from) // not yet migrated
            }
        })
        .collect();
    ids.sort();
    ids.dedup();
    let mut out: Vec<ManuscriptInfo> = ids
        .iter()
        .filter_map(|id| read_manifest(vault, id).ok())
        .map(|m| ManuscriptInfo {
            variants: m.blocks.iter().map(|b| b.variants.len()).sum(),
            blocks: m.blocks.len(),
            id: m.id,
            title: m.title,
        })
        .collect();
    out.sort_by_key(|m| m.title.to_lowercase());
    out
}

fn ms_trash(vault: &Path) -> PathBuf {
    vault.join(".writepad").join("trash").join("manuscripts")
}

/// Move a whole manuscript (every block, take and cut) to the trash.
/// Nothing is erased; [`restore_manuscript`] puts it back.
pub fn trash_manuscript(vault: &Path, id: &str) -> Result<(), String> {
    manifest(vault, id)?; // migrates an old single-file manuscript first
    let to = ms_trash(vault).join(id);
    if to.exists() {
        fs::remove_dir_all(&to).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(ms_trash(vault)).map_err(|e| e.to_string())?;
    fs::rename(ms_dir(vault, id)?, &to).map_err(|e| e.to_string())
}

/// Bring a trashed manuscript back.
pub fn restore_manuscript(vault: &Path, id: &str) -> Result<(), String> {
    let to = ms_dir(vault, id)?;
    if to.exists() {
        return Err("That manuscript is already back.".into());
    }
    fs::rename(ms_trash(vault).join(id), &to).map_err(|_| "Not in the trash.".to_string())
}

// ---------- structural operations ----------

fn block_mut<'a>(m: &'a mut Manifest, block: &str) -> Result<&'a mut BlockMeta, String> {
    m.blocks.iter_mut().find(|b| b.id == block).ok_or_else(|| format!("No block {block:?}"))
}

/// Find the block holding a take.
pub fn block_of<'a>(m: &'a Manifest, take: &str) -> Option<&'a BlockMeta> {
    m.blocks.iter().find(|b| b.variants.iter().any(|v| v.id == take))
}

/// Add a take to a block, after `after` (or at the end). Writes its text first,
/// so nobody ever sees a take without a file.
pub fn add_take(
    vault: &Path,
    ms: &str,
    block: &str,
    after: Option<&str>,
    label: &str,
    author: &str,
    text: &str,
) -> Result<VariantMeta, String> {
    let v = VariantMeta { id: new_id(), label: label.trim().to_string(), author: author.to_string(), created: now() };
    write_atomic(&take_path(vault, ms, &v.id)?, text.as_bytes())?;
    let added = update(vault, ms, |m| {
        let b = block_mut(m, block)?;
        let at = after.and_then(|a| b.variants.iter().position(|x| x.id == a)).map_or(b.variants.len(), |i| i + 1);
        b.variants.insert(at, v.clone());
        Ok(v.clone())
    });
    if added.is_err() {
        let _ = fs::remove_file(take_path(vault, ms, &v.id)?);
    }
    added
}

/// What we remember about a deleted take, so it can be put back.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Trashed {
    pub take: VariantMeta,
    pub block: String,
    pub block_title: String,
    /// The take it sat after (None = first).
    pub after: Option<String>,
    pub was_live: bool,
    pub deleted: u64,
}

fn trash_dir(vault: &Path, ms: &str) -> Result<PathBuf, String> {
    check_id(ms)?;
    Ok(vault.join(".writepad").join("trash").join(ms))
}

/// Move a take's file into the trash with a note of where it was.
fn trash(vault: &Path, ms: &str, record: &Trashed) -> Result<(), String> {
    let dir = trash_dir(vault, ms)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = &record.take.id;
    let from = take_path(vault, ms, id)?;
    if from.exists() {
        fs::rename(&from, dir.join(format!("{id}.md"))).map_err(|e| e.to_string())?;
    }
    write_atomic(&dir.join(format!("{id}.json")), serde_json::to_string_pretty(record).map_err(|e| e.to_string())?.as_bytes())
}

/// Delete a take. It goes to `.writepad/trash/`, and [`restore_take`] brings it back.
pub fn delete_take(vault: &Path, ms: &str, take: &str) -> Result<(), String> {
    let record = update(vault, ms, |m| {
        let b = m.blocks.iter_mut().find(|b| b.variants.iter().any(|v| v.id == take)).ok_or("No such take")?;
        if b.variants.len() == 1 {
            return Err("A block needs at least one take.".into());
        }
        let i = b.variants.iter().position(|v| v.id == take).unwrap();
        let record = Trashed {
            take: b.variants.remove(i),
            block: b.id.clone(),
            block_title: b.title.clone(),
            after: i.checked_sub(1).map(|j| b.variants[j].id.clone()),
            was_live: b.live == take,
            deleted: now(),
        };
        if b.live == take {
            b.live = b.variants.last().unwrap().id.clone();
        }
        Ok(record)
    })?;
    trash(vault, ms, &record)
}

/// Put a deleted take back where it was (and live again, if it was live).
pub fn restore_take(vault: &Path, ms: &str, take: &str) -> Result<(), String> {
    check_id(take)?;
    let dir = trash_dir(vault, ms)?;
    let record: Trashed = serde_json::from_str(&fs::read_to_string(dir.join(format!("{take}.json"))).map_err(|_| "That take isn't in the trash.".to_string())?)
        .map_err(|e| e.to_string())?;
    let file = take_path(vault, ms, take)?;
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::rename(dir.join(format!("{take}.md")), &file).map_err(|e| e.to_string())?;
    update(vault, ms, |m| {
        let b = m.blocks.iter_mut().find(|b| b.id == record.block).ok_or("Its block no longer exists.")?;
        let at = record.after.as_ref().and_then(|a| b.variants.iter().position(|v| &v.id == a)).map_or(0, |i| i + 1);
        b.variants.insert(at.min(b.variants.len()), record.take.clone());
        if record.was_live {
            b.live = take.to_string();
        }
        Ok(())
    })?;
    let _ = fs::remove_file(dir.join(format!("{take}.json")));
    Ok(())
}

/// Deleted takes, newest first.
pub fn trashed(vault: &Path, ms: &str) -> Vec<Trashed> {
    let Ok(entries) = trash_dir(vault, ms).and_then(|d| fs::read_dir(d).map_err(|e| e.to_string())) else { return vec![] };
    let mut out: Vec<Trashed> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a, b| b.deleted.cmp(&a.deleted));
    out
}

pub fn set_live(vault: &Path, ms: &str, take: &str) -> Result<(), String> {
    update(vault, ms, |m| {
        let b = m.blocks.iter_mut().find(|b| b.variants.iter().any(|v| v.id == take)).ok_or("No such take")?;
        b.live = take.to_string();
        Ok(())
    })
}

pub fn rename_take(vault: &Path, ms: &str, take: &str, label: &str) -> Result<(), String> {
    update(vault, ms, |m| {
        let v = m.blocks.iter_mut().flat_map(|b| b.variants.iter_mut()).find(|v| v.id == take).ok_or("No such take")?;
        v.label = label.trim().to_string();
        Ok(())
    })
}

pub fn set_block_title(vault: &Path, ms: &str, block: &str, title: &str) -> Result<(), String> {
    update(vault, ms, |m| {
        block_mut(m, block)?.title = title.to_string();
        Ok(())
    })
}

/// Reorder blocks. Ids missing from `order` keep their relative order at the end,
/// so a reorder based on a stale outline never drops a block someone just added.
pub fn set_order(vault: &Path, ms: &str, order: &[String]) -> Result<(), String> {
    update(vault, ms, |m| {
        let mut rest = std::mem::take(&mut m.blocks);
        for id in order {
            if let Some(i) = rest.iter().position(|b| &b.id == id) {
                m.blocks.push(rest.remove(i));
            }
        }
        m.blocks.extend(rest);
        Ok(())
    })
}

pub fn add_block(vault: &Path, ms: &str, title: &str, after: Option<&str>, author: &str, text: &str) -> Result<BlockMeta, String> {
    let take = VariantMeta { id: new_id(), label: if author.is_empty() { "draft".into() } else { author.to_string() }, author: author.to_string(), created: now() };
    write_atomic(&take_path(vault, ms, &take.id)?, text.as_bytes())?;
    update(vault, ms, |m| {
        let b = BlockMeta { id: new_id(), title: title.to_string(), live: take.id.clone(), variants: vec![take.clone()], raw: false };
        let at = after.and_then(|a| m.blocks.iter().position(|x| x.id == a)).map_or(m.blocks.len(), |i| i + 1);
        m.blocks.insert(at, b.clone());
        Ok(b)
    })
}

/// Remove a block. Its takes go to the trash (their texts are kept there).
pub fn delete_block(vault: &Path, ms: &str, block: &str) -> Result<(), String> {
    let removed = update(vault, ms, |m| {
        let i = m.blocks.iter().position(|b| b.id == block).ok_or("No such block")?;
        Ok(m.blocks.remove(i))
    })?;
    for (i, v) in removed.variants.iter().enumerate() {
        trash(vault, ms, &Trashed {
            take: v.clone(),
            block: removed.id.clone(),
            block_title: removed.title.clone(),
            after: i.checked_sub(1).map(|j| removed.variants[j].id.clone()),
            was_live: removed.live == v.id,
            deleted: now(),
        })?;
    }
    Ok(())
}

pub fn save_cut(vault: &Path, ms: &str, name: &str) -> Result<(), String> {
    update(vault, ms, |m| {
        let cut = Cut {
            name: name.to_string(),
            order: m.blocks.iter().map(|b| b.id.clone()).collect(),
            picks: m.blocks.iter().map(|b| (b.id.clone(), b.live.clone())).collect(),
        };
        m.cuts.retain(|c| c.name != name);
        m.cuts.push(cut);
        Ok(())
    })
}

pub fn apply_cut(vault: &Path, ms: &str, name: &str) -> Result<(), String> {
    update(vault, ms, |m| {
        let cut = m.cuts.iter().find(|c| c.name == name).cloned().ok_or("No such cut")?;
        let mut rest = std::mem::take(&mut m.blocks);
        for id in &cut.order {
            if let Some(i) = rest.iter().position(|b| &b.id == id) {
                m.blocks.push(rest.remove(i));
            }
        }
        m.blocks.extend(rest);
        for b in &mut m.blocks {
            if let Some(pick) = cut.picks.get(&b.id) {
                if b.variants.iter().any(|v| &v.id == pick) {
                    b.live = pick.clone();
                }
            }
        }
        Ok(())
    })
}

pub fn delete_cut(vault: &Path, ms: &str, name: &str) -> Result<(), String> {
    update(vault, ms, |m| {
        m.cuts.retain(|c| c.name != name);
        Ok(())
    })
}

// ---------- import ----------

pub(crate) struct Section {
    /// None for the front matter before the first `## ` heading.
    pub title: Option<String>,
    pub text: String,
}

/// Split markdown on `## ` headings. Returns the sections and whether they
/// were divided by `---` rules.
pub(crate) fn parse_sections(src: &str) -> (Vec<Section>, bool) {
    let mut sections = vec![];
    let mut title: Option<String> = None;
    let mut buf: Vec<&str> = vec![];
    let mut separator = false;

    let mut flush = |title: Option<String>, buf: &mut Vec<&str>, separator: &mut bool| {
        while buf.last().is_some_and(|l| l.trim().is_empty() || l.trim() == "---") {
            if buf.last().is_some_and(|l| l.trim() == "---") {
                *separator = true;
            }
            buf.pop();
        }
        let text = buf.join("\n").trim().to_string();
        buf.clear();
        if title.is_some() || !text.is_empty() {
            sections.push(Section { title, text });
        }
    };

    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            flush(title.take(), &mut buf, &mut separator);
            title = Some(rest.trim().to_string());
        } else {
            buf.push(line);
        }
    }
    flush(title, &mut buf, &mut separator);
    (sections, separator)
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "manuscript".into() } else { slug }
}

fn sections_of(path: &str) -> Result<(Vec<Section>, bool), String> {
    let lower = path.to_lowercase();
    let (mut sections, sep) = if lower.ends_with(".pdf") {
        (crate::pdf::sections(path)?.into_iter().map(|s| Section { title: s.title, text: s.text }).collect(), false)
    } else if lower.ends_with(".epub") {
        let (_, chapters) = crate::epub::chapters(path)?;
        // Copyright pages, "Also by", and the like aren't the book.
        let story = chapters.into_iter().filter(|c| !crate::profile::is_front_matter(&c.title, usize::MAX));
        (story.map(|c| Section { title: Some(c.title), text: c.text }).collect(), false)
    } else {
        parse_sections(&fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
    };
    // Same heading twice in one file means two sections, not two versions.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for s in sections.iter_mut() {
        if let Some(t) = &s.title {
            let n = seen.entry(t.clone()).or_insert(0);
            *n += 1;
            if *n > 1 {
                s.title = Some(format!("{t} ({n})"));
            }
        }
    }
    Ok((sections, sep))
}

/// Write a brand-new manuscript (manifest plus every take's file).
fn create(vault: &Path, title: &str, blocks: Vec<Block>, cuts: Vec<Cut>, separator: bool, id: Option<String>) -> Result<String, String> {
    let _lock = Lock::take(&root(vault).join(".create.lock"))?;
    let id = match id {
        Some(id) => id,
        None => {
            let base = slugify(title);
            let mut id = base.clone();
            let mut n = 2;
            while root(vault).join(&id).exists() || root(vault).join(format!("{id}.json")).exists() {
                id = format!("{base}-{n}");
                n += 1;
            }
            id
        }
    };
    let mut metas = vec![];
    for b in blocks {
        let mut vs = vec![];
        for v in b.variants {
            write_atomic(&take_path(vault, &id, &v.id)?, v.text.as_bytes())?;
            vs.push(VariantMeta { id: v.id, label: v.label, author: v.author, created: v.created });
        }
        metas.push(BlockMeta { id: b.id, title: b.title, variants: vs, live: b.live, raw: b.raw });
    }
    let m = Manifest { version: 2, id: id.clone(), title: title.trim().to_string(), blocks: metas, cuts, separator, rev: 1 };
    write_manifest(vault, &m)?;
    Ok(id)
}

/// Import drafts of one script (markdown, EPUB or PDF). Sections with the same
/// heading line up as takes of one block, oldest first; the newest is live.
pub fn import(vault: &Path, title: &str, paths: &[String]) -> Result<Manuscript, String> {
    let mut blocks: Vec<Block> = vec![];
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut separator = false;

    for path in paths {
        let label = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "draft".into());
        let (sections, sep) = sections_of(path)?;
        separator |= sep;
        for s in sections {
            let key = s.title.clone().unwrap_or_else(|| "\0front".into());
            let i = *index.entry(key).or_insert_with(|| {
                blocks.push(Block {
                    id: new_id(),
                    title: s.title.clone().unwrap_or_else(|| "Front matter".into()),
                    variants: vec![],
                    live: String::new(),
                    raw: s.title.is_none(),
                });
                blocks.len() - 1
            });
            let block = &mut blocks[i];
            if block.variants.last().is_some_and(|v| v.text == s.text) {
                continue; // an unchanged section isn't a new take
            }
            let v = Variant { id: new_id(), label: label.clone(), author: String::new(), text: s.text, created: now() };
            block.live = v.id.clone();
            block.variants.push(v);
        }
    }
    if blocks.is_empty() {
        return Err("No text found in those files.".into());
    }
    let id = create(vault, title, blocks, vec![], separator, None)?;
    load(vault, &id)
}

/// A new manuscript from (title, text) sections, one block each.
pub fn from_sections(vault: &Path, title: &str, label: &str, sections: Vec<(String, String)>) -> Result<String, String> {
    let blocks = sections
        .into_iter()
        .map(|(t, text)| {
            let v = Variant { id: new_id(), label: label.into(), author: String::new(), text, created: now() };
            Block { id: new_id(), title: t, live: v.id.clone(), variants: vec![v], raw: false }
        })
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return Err("Nothing to import.".into());
    }
    create(vault, title, blocks, vec![], false, None)
}

/// One-time upgrade of a v1 manuscript (`<id>.json` with every take's text
/// inline). The old file is kept as `<id>.json.v1`.
fn migrate_v1(vault: &Path, id: &str) -> Result<(), String> {
    let old = root(vault).join(format!("{id}.json"));
    if !old.exists() || ms_dir(vault, id)?.join("manuscript.json").exists() {
        return Ok(());
    }
    let _lock = Lock::take(&root(vault).join(".migrate.lock"))?;
    if ms_dir(vault, id)?.join("manuscript.json").exists() {
        return Ok(()); // another process got here first
    }
    let s = fs::read_to_string(&old).map_err(|e| e.to_string())?;
    let v1: Manuscript = serde_json::from_str(&s).map_err(|e| format!("{}: {e}", old.display()))?;
    create(vault, &v1.title, v1.blocks, v1.cuts, v1.separator, Some(id.to_string()))?;
    fs::rename(&old, old.with_extension("json.v1")).map_err(|e| e.to_string())?;
    Ok(())
}

/// The current cut as (title, text) chapters: outline order, live takes.
/// Front matter blocks come back with no title.
pub fn cut_chapters(m: &Manuscript) -> Vec<(Option<String>, String)> {
    m.blocks
        .iter()
        .map(|b| {
            let text = b.variants.iter().find(|v| v.id == b.live).or(b.variants.last()).map(|v| v.text.clone()).unwrap_or_default();
            ((!b.raw).then(|| b.title.clone()), text)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("writepad-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn splits_on_h2_and_detects_rules() {
        let (s, sep) = parse_sections("# Book\n\nIntro\n\n---\n\n## 1\n\nOne.\n\n---\n\n## 2\n\nTwo.\n");
        assert!(sep);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].title, None);
        assert_eq!(s[1].text, "One.");
    }

    #[test]
    fn trash_and_restore_whole_manuscript() {
        let vault = temp_vault("trash-ms");
        let id = from_sections(&vault, "Book", "draft_1", vec![("One".into(), "Hello.".into())]).unwrap();
        assert_eq!(list(&vault).len(), 1);
        trash_manuscript(&vault, &id).unwrap();
        assert!(list(&vault).is_empty());
        restore_manuscript(&vault, &id).unwrap();
        assert_eq!(load(&vault, &id).unwrap().blocks[0].variants[0].text.trim(), "Hello.");
    }

    #[test]
    fn migrates_v1_and_keeps_backup() {
        let vault = temp_vault("migrate");
        let v1 = r#"{"id":"book","title":"Book","separator":true,"cuts":[],"blocks":[
            {"id":"b1","title":"1","live":"v2","raw":false,"variants":[
              {"id":"v1","label":"draft_1","text":"Old.","created":1},
              {"id":"v2","label":"draft_2","text":"New.","created":2}]}]}"#;
        fs::create_dir_all(root(&vault)).unwrap();
        fs::write(root(&vault).join("book.json"), v1).unwrap();
        let m = load(&vault, "book").unwrap();
        assert_eq!(m.blocks[0].variants[1].text, "New.");
        assert_eq!(m.blocks[0].live, "v2");
        assert!(root(&vault).join("book.json.v1").exists());
        assert_eq!(list(&vault).len(), 1);
        fs::remove_dir_all(vault).unwrap();
    }

    #[test]
    fn deleted_takes_go_to_trash_and_come_back() {
        let vault = temp_vault("trash");
        let src = vault.join("draft.md");
        fs::write(&src, "## 1\n\nOriginal.\n").unwrap();
        let m = import(&vault, "Book", &[src.to_string_lossy().into()]).unwrap();
        let (b, original) = (m.blocks[0].id.clone(), m.blocks[0].live.clone());
        add_take(&vault, &m.id, &b, Some(&original), "fork", "", "Fork.").unwrap();
        delete_take(&vault, &m.id, &original).unwrap();
        assert_eq!(load(&vault, &m.id).unwrap().blocks[0].variants.len(), 1);
        assert_eq!(trashed(&vault, &m.id).len(), 1);
        restore_take(&vault, &m.id, &original).unwrap();
        let back = load(&vault, &m.id).unwrap();
        assert_eq!(back.blocks[0].variants[0].text, "Original.");
        assert_eq!(back.blocks[0].live, original, "was live, is live again");
        assert!(trashed(&vault, &m.id).is_empty());
        fs::remove_dir_all(vault).unwrap();
    }

    /// Many writers, many threads, one manuscript: nothing gets lost.
    #[test]
    fn concurrent_takes_are_all_kept() {
        let vault = temp_vault("concurrent");
        let src = vault.join("draft.md");
        fs::write(&src, "## 1\n\nOne.\n\n## 2\n\nTwo.\n").unwrap();
        let m = import(&vault, "Race", &[src.to_string_lossy().into()]).unwrap();
        let (ms, block) = (m.id.clone(), m.blocks[0].id.clone());
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (vault, ms, block) = (vault.clone(), ms.clone(), block.clone());
                std::thread::spawn(move || {
                    for j in 0..10 {
                        let agent = format!("agent-{i}");
                        let t = add_take(&vault, &ms, &block, None, &format!("{agent} #{j}"), &agent, &format!("text {i}.{j}")).unwrap();
                        write_take(&vault, &ms, &t.id, &format!("revised {i}.{j}")).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let m = load(&vault, &ms).unwrap();
        assert_eq!(m.blocks[0].variants.len(), 1 + 80);
        assert!(m.blocks[0].variants[1..].iter().all(|v| v.text.starts_with("revised")));
        assert_eq!(m.rev, 1 + 80);
        fs::remove_dir_all(vault).unwrap();
    }
}
