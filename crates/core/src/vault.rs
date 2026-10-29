//! Notes: plain markdown files in the vault folder, readable by Obsidian.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Serialize)]
pub struct NoteEntry {
    /// Path relative to the vault, with forward slashes.
    pub path: String,
    pub name: String,
}

/// Resolve a vault-relative path, refusing anything that escapes the vault.
pub fn resolve(vault: &Path, rel: &str) -> Result<PathBuf, String> {
    let p = Path::new(rel);
    let safe = p
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !safe || rel.is_empty() {
        return Err(format!("Not a path inside the vault: {rel}"));
    }
    Ok(vault.join(p))
}

pub fn list_notes(vault: &Path) -> Vec<NoteEntry> {
    let mut notes: Vec<NoteEntry> = WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| {
            let rel = e.path().strip_prefix(vault).ok()?;
            Some(NoteEntry {
                path: rel.to_string_lossy().replace('\\', "/"),
                name: rel.file_stem()?.to_string_lossy().into_owned(),
            })
        })
        .collect();
    notes.sort_by_key(|n| n.path.to_lowercase());
    notes
}

pub fn read(vault: &Path, rel: &str) -> Result<String, String> {
    fs::read_to_string(resolve(vault, rel)?).map_err(|e| e.to_string())
}

pub fn write(vault: &Path, rel: &str, text: &str) -> Result<(), String> {
    crate::write_atomic(&resolve(vault, rel)?, text.as_bytes())
}

/// Append to a note under a lock, so several writers can add to one note
/// (a shared ideas file, a log) without overwriting each other.
pub fn append(vault: &Path, rel: &str, text: &str) -> Result<(), String> {
    let path = resolve(vault, rel)?;
    let _lock = crate::manuscript::Lock::take(&vault.join(".writepad").join(".notes.lock"))?;
    let mut current = fs::read_to_string(&path).unwrap_or_default();
    if !current.is_empty() && !current.ends_with("\n\n") {
        current.push_str(if current.ends_with('\n') { "\n" } else { "\n\n" });
    }
    current.push_str(text.trim_end());
    current.push('\n');
    crate::write_atomic(&path, current.as_bytes())
}

/// Create `<name>.md` (or open it if it already exists) and return its relative path.
pub fn create(vault: &Path, name: &str) -> Result<String, String> {
    let name = name.trim().trim_end_matches(".md");
    let rel = format!("{name}.md");
    let path = resolve(vault, &rel)?;
    if !path.exists() {
        let title = Path::new(name).file_name().map(|s| s.to_string_lossy().into_owned());
        write(vault, &rel, &format!("# {}\n\n", title.unwrap_or_default()))?;
    }
    Ok(rel)
}

/// A deleted note, kept in `.writepad/trash/notes/<id>/` until restored.
#[derive(Serialize, Deserialize)]
pub struct TrashedNote {
    pub id: String,
    pub path: String,
    pub deleted: u64,
    /// Timeline events that followed this one (re-linked on restore).
    #[serde(default)]
    pub successors: Vec<String>,
}

fn note_trash(vault: &Path) -> PathBuf {
    vault.join(".writepad").join("trash").join("notes")
}

/// Move a note to the trash. Returns the trash id for [`restore`].
pub fn trash(vault: &Path, rel: &str, successors: Vec<String>) -> Result<String, String> {
    let from = resolve(vault, rel)?;
    if !from.is_file() {
        return Err(format!("No note {rel}"));
    }
    let id = crate::manuscript::new_id();
    let dir = note_trash(vault).join(&id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::rename(&from, dir.join("note.md")).map_err(|e| e.to_string())?;
    let record = TrashedNote { id: id.clone(), path: rel.to_string(), deleted: crate::manuscript::now(), successors };
    crate::write_atomic(&dir.join("record.json"), serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?.as_bytes())?;
    Ok(id)
}

/// Put a trashed note back at its old path. Returns the record.
pub fn restore(vault: &Path, id: &str) -> Result<TrashedNote, String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Bad trash id".into());
    }
    let dir = note_trash(vault).join(id);
    let record: TrashedNote = serde_json::from_str(&fs::read_to_string(dir.join("record.json")).map_err(|_| "Not in the trash.".to_string())?)
        .map_err(|e| e.to_string())?;
    let to = resolve(vault, &record.path)?;
    if to.exists() {
        return Err(format!("{} exists again; rename it first.", record.path));
    }
    if let Some(d) = to.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::rename(dir.join("note.md"), &to).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    Ok(record)
}

/// Where pictures dropped or pasted into a page are kept, as in Obsidian's default.
pub const ATTACHMENTS: &str = "attachments";

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp"];

fn image_ext(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_string_lossy().to_lowercase();
    IMAGE_EXTS.contains(&ext.as_str()).then_some(ext)
}

/// Save an image into the vault's attachments folder under a name not already
/// taken, and return its vault-relative path for a `![](...)` link.
pub fn save_image(vault: &Path, name: &str, data: &[u8]) -> Result<String, String> {
    let ext = image_ext(name).ok_or_else(|| format!("Not an image WRITEpad can show: {name}"))?;
    let stem: String = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let stem = stem.trim_matches('-');
    let stem = if stem.is_empty() { "image" } else { stem };
    let dir = vault.join(ATTACHMENTS);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = format!("{stem}.{ext}");
    let mut n = 2;
    while dir.join(&file).exists() {
        file = format!("{stem}-{n}.{ext}");
        n += 1;
    }
    crate::write_atomic(&dir.join(&file), data)?;
    Ok(format!("{ATTACHMENTS}/{file}"))
}

/// Copy an image from anywhere on disk into the vault (see `save_image`).
pub fn import_image(vault: &Path, from: &Path) -> Result<String, String> {
    let name = from.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    image_ext(&name).ok_or_else(|| format!("Not an image WRITEpad can show: {name}"))?;
    let data = fs::read(from).map_err(|e| e.to_string())?;
    save_image(vault, &name, &data)
}

/// An image's bytes, for showing it in the page. Only images, only inside the vault.
pub fn read_image(vault: &Path, rel: &str) -> Result<Vec<u8>, String> {
    image_ext(rel).ok_or_else(|| format!("Not an image: {rel}"))?;
    fs::read(resolve(vault, rel)?).map_err(|e| e.to_string())
}
