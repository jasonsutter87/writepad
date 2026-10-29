mod dictate;
mod settings;
mod tts_setup;
mod watch;

use dictate::{Dictation, Mic, Whisper};
use manuscript::{Manuscript, ManuscriptInfo};
use narrate::{Lexicon, Worker};
use writepad_core::{comments, diff, export, history, manuscript, narrate, profile, script, search, suggest, vault, world};
use settings::Settings;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{ipc::Response, AppHandle, Emitter, Manager, State};

struct AppState {
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    cache_dir: PathBuf,
    worker: Mutex<Option<Worker>>,
    /// OS data dir (~/Library/Application Support), where murmur keeps its models and key.
    data_dir: PathBuf,
    mic: Mic,
    whisper: Arc<Whisper>,
    /// Watches the vault so edits from agents (or anything else) show up live.
    watcher: Mutex<Option<watch::Watcher>>,
}

impl AppState {
    fn vault(&self) -> Result<PathBuf, String> {
        self.settings
            .lock()
            .unwrap()
            .vault
            .clone()
            .map(PathBuf::from)
            .ok_or_else(|| "Open a vault folder first.".to_string())
    }

    /// Run `f` against a live worker, starting (or restarting) it as needed.
    fn with_worker<T>(&self, f: impl FnOnce(&mut Worker) -> Result<T, String>) -> Result<T, String> {
        let mut slot = self.worker.lock().unwrap();
        if !slot.as_mut().is_some_and(Worker::alive) {
            let s = self.settings.lock().unwrap().clone();
            *slot = Some(Worker::start(&s.python, &s.model_dir, &self.cache_dir)?);
        }
        f(slot.as_mut().unwrap())
    }
}

/// Blocking work (file IO, the TTS worker) runs off the main thread.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
    let mut current = state.settings.lock().unwrap();
    if current.python != settings.python || current.model_dir != settings.model_dir {
        *state.worker.lock().unwrap() = None;
    }
    settings::save(&state.settings_path, &settings)?;
    let vault_changed = current.vault != settings.vault;
    *current = settings;
    drop(current);
    if vault_changed {
        restart_watcher(&app, &state);
    }
    Ok(())
}

fn restart_watcher(app: &AppHandle, state: &AppState) {
    let vault = state.settings.lock().unwrap().vault.clone();
    *state.watcher.lock().unwrap() = vault.and_then(|v| watch::start(app.clone(), PathBuf::from(v)).map_err(|e| eprintln!("watch: {e}")).ok());
}

#[tauri::command]
fn list_notes(state: State<AppState>) -> Result<Vec<vault::NoteEntry>, String> {
    Ok(vault::list_notes(&state.vault()?))
}

#[tauri::command]
fn search_vault(state: State<AppState>, query: String) -> Result<Vec<search::Hit>, String> {
    Ok(search::search(&state.vault()?, &query, 200))
}

#[tauri::command]
fn read_note(state: State<AppState>, path: String) -> Result<String, String> {
    vault::read(&state.vault()?, &path)
}

#[tauri::command]
fn write_note(state: State<AppState>, path: String, text: String) -> Result<(), String> {
    vault::write(&state.vault()?, &path, &text)
}

#[tauri::command]
fn create_note(state: State<AppState>, name: String) -> Result<String, String> {
    vault::create(&state.vault()?, &name)
}

#[tauri::command]
fn list_manuscripts(state: State<AppState>) -> Result<Vec<ManuscriptInfo>, String> {
    Ok(manuscript::list(&state.vault()?))
}

/// Copy a picture chosen on disk into the vault's attachments folder.
#[tauri::command]
fn import_image(state: State<AppState>, path: String) -> Result<String, String> {
    vault::import_image(&state.vault()?, std::path::Path::new(&path))
}

/// Save a pasted or dropped picture: the raw bytes are the body, its file name an `x-name` header.
#[tauri::command]
fn save_image(state: State<AppState>, request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("Expected the image's bytes.".into());
    };
    let name = request.headers().get("x-name").and_then(|v| v.to_str().ok()).unwrap_or("image.png");
    vault::save_image(&state.vault()?, name, data)
}

#[tauri::command]
fn read_image(state: State<AppState>, path: String) -> Result<Response, String> {
    vault::read_image(&state.vault()?, &path).map(Response::new)
}

#[tauri::command]
fn load_manuscript(state: State<AppState>, id: String) -> Result<Manuscript, String> {
    manuscript::load(&state.vault()?, &id)
}

/// Replace one take's text (only that take's file is written).
#[tauri::command]
fn write_take(state: State<AppState>, manuscript: String, take: String, text: String) -> Result<(), String> {
    manuscript::write_take(&state.vault()?, &manuscript, &take, &text)
}

/// Structural changes all return the fresh manuscript, so the app always
/// reflects what's on disk, including other writers' changes.
fn changed(state: &AppState, id: &str, f: impl FnOnce(&std::path::Path) -> Result<(), String>) -> Result<Manuscript, String> {
    let vault = state.vault()?;
    f(&vault)?;
    manuscript::load(&vault, id)
}

#[tauri::command]
fn add_take(state: State<AppState>, manuscript: String, block: String, after: Option<String>, label: String, text: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::add_take(v, &manuscript, &block, after.as_deref(), &label, "", &text).map(|_| ()))
}

#[tauri::command]
fn delete_take(state: State<AppState>, manuscript: String, take: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::delete_take(v, &manuscript, &take))
}

#[tauri::command]
fn restore_take(state: State<AppState>, manuscript: String, take: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::restore_take(v, &manuscript, &take))
}

#[tauri::command]
fn set_live(state: State<AppState>, manuscript: String, take: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_live(v, &manuscript, &take))
}

#[tauri::command]
fn rename_take(state: State<AppState>, manuscript: String, take: String, label: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::rename_take(v, &manuscript, &take, &label))
}

#[tauri::command]
fn set_block_props(state: State<AppState>, manuscript: String, block: String, props: Vec<manuscript::Prop>) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_block_props(v, &manuscript, &block, props))
}

#[tauri::command]
fn set_block_out(state: State<AppState>, manuscript: String, block: String, out: bool) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_block_out(v, &manuscript, &block, out))
}

#[tauri::command]
fn restore_cut(state: State<AppState>, manuscript: String, cut: manuscript::Cut, at: usize) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::restore_cut(v, &manuscript, cut, at))
}

#[tauri::command]
fn set_target(state: State<AppState>, manuscript: String, target: manuscript::Target) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_target(v, &manuscript, target))
}

#[tauri::command]
fn set_block_title(state: State<AppState>, manuscript: String, block: String, title: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_block_title(v, &manuscript, &block, &title))
}

#[tauri::command]
fn set_order(state: State<AppState>, manuscript: String, order: Vec<String>) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_order(v, &manuscript, &order))
}

/// A new, empty chapter after `after` (or at the end).
#[tauri::command]
fn add_block(state: State<AppState>, manuscript: String, title: String, after: Option<String>) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::add_block(v, &manuscript, &title, after.as_deref(), "", "").map(|_| ()))
}

#[tauri::command]
fn delete_block(state: State<AppState>, manuscript: String, block: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::delete_block(v, &manuscript, &block))
}

#[tauri::command]
fn list_comments(state: State<AppState>, manuscript: String) -> Result<Vec<comments::Comment>, String> {
    comments::list(&state.vault()?, &manuscript)
}

/// Save one take's comments, with offsets moved by the latest edits.
#[tauri::command]
fn set_comments(state: State<AppState>, manuscript: String, take: String, comments: Vec<comments::Comment>) -> Result<(), String> {
    writepad_core::comments::set_for_take(&state.vault()?, &manuscript, &take, comments)
}

#[tauri::command]
fn restore_block(state: State<AppState>, manuscript: String, block: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::restore_block(v, &manuscript, &block))
}

#[tauri::command]
fn cut_op(state: State<AppState>, manuscript: String, op: String, name: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| match op.as_str() {
        "save" => manuscript::save_cut(v, &manuscript, &name),
        "apply" => manuscript::apply_cut(v, &manuscript, &name),
        "delete" => manuscript::delete_cut(v, &manuscript, &name),
        _ => Err(format!("Unknown cut op {op}")),
    })
}

#[tauri::command]
async fn import_drafts(app: AppHandle, title: String, paths: Vec<String>) -> Result<Manuscript, String> {
    blocking(app, move |st| manuscript::import(&st.vault()?, &title, &paths)).await
}


#[tauri::command]
fn diff_words(old: String, new: String) -> Vec<diff::Op> {
    diff::words(&old, &new)
}

/// Start the narration worker (loading the model) and list its voices.
#[tauri::command]
async fn voices(app: AppHandle) -> Result<Vec<String>, String> {
    blocking(app, |st| st.with_worker(|w| Ok(w.voices.clone()))).await
}

/// Install Kokoro into a private venv and download the models, then point
/// Settings at them. Progress arrives as "narration-setup" events.
#[tauri::command]
async fn setup_narration(app: AppHandle) -> Result<Settings, String> {
    let emitter = app.clone();
    blocking(app, move |st| {
        let preferred = st.settings.lock().unwrap().python.clone();
        let root = emitter.path().app_data_dir().map_err(|e| e.to_string())?.join("tts");
        let done = tts_setup::install(&root, &preferred, |message| {
            let _ = emitter.emit("narration-setup", tts_setup::Progress { message });
        })?;
        *st.worker.lock().unwrap() = None;
        let mut s = st.settings.lock().unwrap();
        s.python = done.python;
        s.model_dir = done.model_dir;
        settings::save(&st.settings_path, &s)?;
        Ok(s.clone())
    })
    .await
}

#[tauri::command]
async fn synth(app: AppHandle, text: String, voice: String, speed: f32) -> Result<Response, String> {
    blocking(app, move |st| {
        let lexicon = narrate::load_lexicon(st.vault().ok().as_deref());
        let wav = st.with_worker(|w| narrate::synth(w, &st.cache_dir, &text, &voice, speed, &lexicon))?;
        Ok(Response::new(wav))
    })
    .await
}

#[tauri::command]
fn get_lexicon(state: State<AppState>) -> Lexicon {
    narrate::load_lexicon(state.vault().ok().as_deref())
}

#[tauri::command]
fn set_lexicon(state: State<AppState>, lexicon: Lexicon) -> Result<(), String> {
    let path = narrate::lexicon_path(&state.vault()?);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&lexicon).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[tauri::command]
async fn dictate_start(app: AppHandle) -> Result<(), String> {
    blocking(app, |st| {
        st.mic.start()?;
        // Load whisper while the writer is still talking.
        let s = st.settings.lock().unwrap().clone();
        let (whisper, models) = (st.whisper.clone(), dictate::murmur_dir(&st.data_dir).join("models"));
        std::thread::spawn(move || {
            if let Err(e) = whisper.get(&models, &s.dictation_model, &s.dictation_language) {
                eprintln!("whisper: {e}");
            }
        });
        Ok(())
    })
    .await
}

/// Stop recording and return the text to insert. `before` is the text just
/// before the cursor, so the polish pass can continue it naturally.
#[tauri::command]
async fn dictate_stop(app: AppHandle, before: String) -> Result<Dictation, String> {
    blocking(app, move |st| {
        let none = |warning: Option<String>| Dictation { text: String::new(), polished: false, warning };
        let Some(samples) = st.mic.stop() else { return Ok(none(None)) };
        let s = st.settings.lock().unwrap().clone();
        let models = dictate::murmur_dir(&st.data_dir).join("models");
        let whisper = st.whisper.get(&models, &s.dictation_model, &s.dictation_language)?;
        let raw = dictate::transcribe(&whisper, &samples)?;
        if raw.is_empty() || !s.dictation_polish {
            return Ok(Dictation { text: raw, polished: false, warning: None });
        }
        let Some(key) = dictate::api_key(&st.data_dir) else {
            return Ok(Dictation {
                text: raw,
                polished: false,
                warning: Some("Polish needs an Anthropic API key (murmur's key file or ANTHROPIC_API_KEY). Inserted the raw transcript.".into()),
            });
        };
        Ok(match dictate::polish(&key, &before, &raw) {
            Ok(text) => Dictation { text, polished: true, warning: None },
            Err(e) => Dictation { text: raw, polished: false, warning: Some(format!("{e}. Inserted the raw transcript.")) },
        })
    })
    .await
}

#[tauri::command]
async fn dictate_cancel(app: AppHandle) -> Result<(), String> {
    blocking(app, |st| {
        st.mic.stop();
        Ok(())
    })
    .await
}

#[tauri::command]
fn history_status(state: State<AppState>) -> Result<history::Status, String> {
    Ok(history::status(&state.vault()?))
}

#[tauri::command]
async fn history_enable(app: AppHandle) -> Result<(), String> {
    blocking(app, |st| history::enable(&st.vault()?)).await
}

#[tauri::command]
async fn history_snapshot(app: AppHandle, message: String) -> Result<Option<history::Snapshot>, String> {
    blocking(app, move |st| history::snapshot(&st.vault()?, &message)).await
}

#[tauri::command]
async fn history_log(app: AppHandle) -> Result<Vec<history::Snapshot>, String> {
    blocking(app, |st| history::log(&st.vault()?, 200)).await
}

#[tauri::command]
async fn note_history(app: AppHandle, path: String) -> Result<Vec<history::Entry>, String> {
    blocking(app, move |st| history::note_history(&st.vault()?, &path, 100)).await
}

#[tauri::command]
async fn take_history(app: AppHandle, manuscript: String, take: String) -> Result<Vec<history::Entry>, String> {
    blocking(app, move |st| history::take_history(&st.vault()?, &manuscript, &take, 100)).await
}

#[tauri::command]
async fn history_restore_all(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |st| history::restore_all(&st.vault()?, &id)).await
}

#[tauri::command]
async fn export_book(app: AppHandle, book: export::Book, format: String, path: String) -> Result<(), String> {
    blocking(app, move |st| export::write_book(&book, &format, std::path::Path::new(&path), &st.cache_dir.join("export"))).await
}

/// Narrate one chapter to an MP3 file. Sentences come pre-split from the
/// frontend (the same splitter the Listen button uses) and are rendered
/// through the narration cache.
#[tauri::command]
async fn export_audio_chapter(
    app: AppHandle,
    segments: Vec<export::Segment>,
    speed: f32,
    path: String,
    title: String,
    album: String,
    artist: String,
) -> Result<(), String> {
    blocking(app, move |st| {
        let lexicon = narrate::load_lexicon(st.vault().ok().as_deref());
        let mut wavs = Vec::with_capacity(segments.len());
        for seg in &segments {
            let wav = st.with_worker(|w| narrate::synth(w, &st.cache_dir, &seg.text, &seg.voice, speed, &lexicon))?;
            wavs.push((wav, seg.pause));
        }
        export::mp3(wavs, std::path::Path::new(&path), &title, &album, &artist)
    })
    .await
}

#[tauri::command]
async fn world_graph(app: AppHandle) -> Result<world::Graph, String> {
    blocking(app, |st| Ok(world::graph(&st.vault()?))).await
}

/// The timeline, with "told earlier" measured against a manuscript (or one
/// of its cuts) when given.
#[tauri::command]
async fn world_timeline(app: AppHandle, manuscript: Option<String>, cut: Option<String>) -> Result<Vec<world::TimelineEvent>, String> {
    blocking(app, move |st| world::timeline_read(&st.vault()?, manuscript.as_deref().map(|m| (m, cut.as_deref())))).await
}

#[derive(serde::Serialize)]
struct Telling {
    id: String,
    title: String,
    cuts: Vec<String>,
}

/// Tellings the timeline can be read against: each manuscript and its cuts.
#[tauri::command]
async fn world_tellings(app: AppHandle) -> Result<Vec<Telling>, String> {
    blocking(app, |st| {
        let v = st.vault()?;
        Ok(manuscript::list(&v)
            .into_iter()
            .map(|m| Telling { cuts: manuscript::manifest(&v, &m.id).map(|x| x.cuts.into_iter().map(|c| c.name).collect()).unwrap_or_default(), id: m.id, title: m.title })
            .collect())
    })
    .await
}

#[tauri::command]
async fn world_entities(app: AppHandle) -> Result<Vec<world::Entity>, String> {
    blocking(app, |st| Ok(world::entities(&st.vault()?))).await
}

#[tauri::command]
async fn world_entity(app: AppHandle, path: String) -> Result<Option<world::EntityInfo>, String> {
    blocking(app, move |st| Ok(world::entity_info(&st.vault()?, &path).ok())).await
}

#[tauri::command]
async fn library_report(app: AppHandle, manuscript: Option<String>) -> Result<profile::Report, String> {
    blocking(app, move |st| Ok(profile::report(&st.vault()?, manuscript.as_deref()))).await
}

/// Measure EPUBs into the library. Returns the ones that failed, with why.
#[tauri::command]
async fn library_add(app: AppHandle, paths: Vec<String>) -> Result<Vec<String>, String> {
    blocking(app, move |st| {
        let v = st.vault()?;
        Ok(paths.iter().filter_map(|p| profile::add(&v, p, None, None).err()).collect())
    })
    .await
}

#[tauri::command]
async fn library_remove(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |st| profile::remove(&st.vault()?, &id)).await
}

#[tauri::command]
async fn library_set_weights(app: AppHandle, weights: std::collections::BTreeMap<String, f64>) -> Result<(), String> {
    blocking(app, move |st| profile::set_weights(&st.vault()?, &weights)).await
}

#[tauri::command]
async fn script_add(app: AppHandle, source: String) -> Result<script::ScriptSummary, String> {
    blocking(app, move |st| Ok(script::summarize(&script::add(&st.vault()?, source.trim(), None, None)?))).await
}

#[tauri::command]
async fn script_remove(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |st| script::remove(&st.vault()?, &id)).await
}

#[tauri::command]
async fn scripts_report(app: AppHandle) -> Result<Vec<script::ScriptSummary>, String> {
    blocking(app, move |st| Ok(script::report(&st.vault()?))).await
}

#[tauri::command]
async fn adaptation(app: AppHandle, book: String, script_id: String) -> Result<script::Adaptation, String> {
    blocking(app, move |st| script::adapt(&st.vault()?, &book, &script_id)).await
}

#[tauri::command]
async fn script_open(app: AppHandle, id: String) -> Result<String, String> {
    blocking(app, move |st| script::open_as_manuscript(&st.vault()?, &id)).await
}

/// Who reads which character, per manuscript or note: `.writepad/cast.json`.
fn cast_path(vault: &std::path::Path) -> PathBuf {
    vault.join(".writepad").join("cast.json")
}

type Casts = std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>;

#[tauri::command]
async fn get_cast(app: AppHandle, scope: String) -> Result<std::collections::BTreeMap<String, String>, String> {
    blocking(app, move |st| {
        let all: Casts = std::fs::read_to_string(cast_path(&st.vault()?)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Ok(all.get(&scope).cloned().unwrap_or_default())
    })
    .await
}

#[tauri::command]
async fn set_cast(app: AppHandle, scope: String, cast: std::collections::BTreeMap<String, String>) -> Result<(), String> {
    blocking(app, move |st| {
        let path = cast_path(&st.vault()?);
        let mut all: Casts = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        all.insert(scope, cast);
        writepad_core::write_atomic(&path, serde_json::to_string_pretty(&all).unwrap().as_bytes())
    })
    .await
}

#[tauri::command]
async fn cadence(app: AppHandle, text: String, manuscript: Option<String>) -> Result<profile::Cadence, String> {
    blocking(app, move |st| {
        let vault = st.vault()?;
        let mine = manuscript.and_then(|id| manuscript::manifest(&vault, &id).ok()).and_then(|m| m.target);
        Ok(profile::cadence(&text, profile::target(&vault).as_ref(), mine.as_ref()))
    })
    .await
}

#[tauri::command]
async fn world_scene(app: AppHandle, manuscript: String, block: String) -> Result<world::SceneInfo, String> {
    blocking(app, move |st| world::scene_info(&st.vault()?, &manuscript, &block)).await
}

/// Characters and locations found in a manuscript by plain rules (no AI).
#[tauri::command]
async fn world_suggest(app: AppHandle, manuscript: String) -> Result<suggest::Suggestions, String> {
    blocking(app, move |st| suggest::suggest(&st.vault()?, &manuscript)).await
}

/// Add the picked suggestions to the world as drafts, plus an optional timeline skeleton.
#[tauri::command]
async fn world_suggest_apply(app: AppHandle, manuscript: String, picks: Vec<suggest::Suggestion>, timeline: bool) -> Result<Vec<String>, String> {
    blocking(app, move |st| suggest::apply(&st.vault()?, &manuscript, &picks, timeline)).await
}

#[tauri::command]
async fn world_import_csv(app: AppHandle, path: String) -> Result<suggest::CsvImport, String> {
    blocking(app, move |st| {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        suggest::import_csv(&st.vault()?, &text)
    })
    .await
}

#[tauri::command]
fn world_add(state: State<AppState>, kind: String, name: String) -> Result<String, String> {
    world::add(&state.vault()?, &kind, &name, &[], "")
}

/// A new timeline event after `after`, spliced in ahead of `before`; with a
/// scene, it's linked to that manuscript block.
#[tauri::command]
fn world_insert_event(
    state: State<AppState>,
    name: String,
    after: Option<String>,
    before: Option<String>,
    when: Option<String>,
    scene: Option<(String, String)>,
    timeline: Option<String>,
) -> Result<String, String> {
    let vault = state.vault()?;
    let mut props = vec![];
    if let Some(t) = timeline.filter(|t| !t.trim().is_empty()) {
        props.push(("timeline".to_string(), serde_json::to_string(t.trim()).unwrap()));
    }
    if let Some(w) = when.filter(|w| !w.trim().is_empty()) {
        props.push(("when".to_string(), serde_json::to_string(w.trim()).unwrap()));
    }
    if let Some((ms, block)) = scene {
        let link = format!("[[{}]]", world::scene_ref(&vault, &ms, &block)?);
        props.push(("scenes".to_string(), format!("[{}]", serde_json::to_string(&link).unwrap())));
    }
    world::insert_event(&vault, &name, after.as_deref(), before.as_deref(), props, "")
}

#[tauri::command]
fn world_move_event(state: State<AppState>, path: String, after: Option<String>, before: Option<String>) -> Result<(), String> {
    world::move_event(&state.vault()?, &path, after.as_deref(), before.as_deref())
}

#[tauri::command]
fn world_delete_event(state: State<AppState>, path: String) -> Result<String, String> {
    world::delete_event(&state.vault()?, &path)
}

/// Move any note to the trash; returns the id that `restore_note` takes.
#[tauri::command]
fn delete_note(state: State<AppState>, path: String) -> Result<String, String> {
    world::delete_note(&state.vault()?, &path)
}

/// A blank manuscript or screenplay; returns its id.
#[tauri::command]
fn create_manuscript(state: State<AppState>, title: String, screenplay: bool) -> Result<String, String> {
    manuscript::new_blank(&state.vault()?, &title, screenplay)
}

/// Move a whole manuscript to the trash (restorable with `restore_manuscript`).
#[tauri::command]
fn delete_manuscript(state: State<AppState>, id: String) -> Result<(), String> {
    manuscript::trash_manuscript(&state.vault()?, &id)
}

#[tauri::command]
fn restore_manuscript(state: State<AppState>, id: String) -> Result<(), String> {
    manuscript::restore_manuscript(&state.vault()?, &id)
}

#[tauri::command]
fn restore_note(state: State<AppState>, id: String) -> Result<String, String> {
    world::restore_note(&state.vault()?, &id)
}

/// The website, for help pages and the blog. Change it here when WRITEpad
/// gets its own domain.
const SITE: &str = "https://writepad-app.netlify.app";
const ISSUES: &str = "https://github.com/jasonsutter87/writepad/issues";

/// Open a page of the website in the browser: a help topic (`"cuts"`), the
/// help index (`""`), or the blog (`"blog"`). Only these, so a page can't ask
/// the app to open anything else.
fn open_site(page: &str) -> Result<(), String> {
    open_url(&site_url(page).ok_or_else(|| format!("No help page {page:?}"))?)
}

fn site_url(page: &str) -> Option<String> {
    Some(match page {
        "" => format!("{SITE}/docs/"),
        "blog" => format!("{SITE}/blog/"),
        "issues" => ISSUES.to_string(),
        // Narration setup sends people here when they have no Python 3.10+.
        "python" => "https://www.python.org/downloads/".to_string(),
        p if p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') => format!("{SITE}/docs/{p}/"),
        _ => return None,
    })
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ()).map_err(|e| format!("Couldn't open {url}: {e}"))
}

#[tauri::command]
fn open_help(page: String) -> Result<(), String> {
    open_site(&page)
}

/// The demo vault that ships inside the app, as a folder the writer owns:
/// `Documents/WRITEpad Demo`. An existing copy is reused, so edits survive.
#[tauri::command]
async fn open_demo(app: AppHandle) -> Result<String, String> {
    let paths = app.path().clone();
    blocking(app, move |_| {
        let from = paths.resolve("demo", tauri::path::BaseDirectory::Resource).map_err(|e| e.to_string())?;
        let to = paths.document_dir().map_err(|e| e.to_string())?.join("WRITEpad Demo");
        if !to.join(".writepad").exists() {
            copy_dir(&from, &to).map_err(|e| format!("Copying the demo: {e}"))?;
        }
        Ok(to.to_string_lossy().into_owned())
    })
    .await
}

/// A folder for a first book: `Documents/WRITEpad`, made if it isn't there.
#[tauri::command]
fn new_book_folder(app: AppHandle) -> Result<String, String> {
    let dir = app.path().document_dir().map_err(|e| e.to_string())?.join("WRITEpad");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().into_owned())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".lock") {
            continue;
        }
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to.join(&name))?;
        } else {
            std::fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// The app menu. Undo/Redo are our own items rather than the system ones:
/// the system Undo asks WebKit to undo, but the editor keeps its own history,
/// so the frontend handles these (see "menu" in main.ts).
fn app_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let app_menu = Submenu::with_items(
        app,
        "WRITEpad",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &MenuItem::with_id(app, "undo", "Undo", true, Some("CmdOrCtrl+Z"))?,
            &MenuItem::with_id(app, "redo", "Redo", true, Some("CmdOrCtrl+Shift+Z"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[&MenuItem::with_id(app, "focus", "Focus mode", true, Some("CmdOrCtrl+Shift+D"))?, &PredefinedMenuItem::fullscreen(app, None)?],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::maximize(app, None)?, &PredefinedMenuItem::close_window(app, None)?],
    )?;
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &MenuItem::with_id(app, "help", "WRITEpad Help", true, Some("CmdOrCtrl+?"))?,
            &MenuItem::with_id(app, "help:blog", "Writing blog", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "help:issues", "Report a bug…", true, None::<&str>)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &view, &window, &help])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Signed self-updates from GitHub Releases (see RELEASING.md).
            #[cfg(desktop)]
            app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let cache_dir = app.path().app_cache_dir()?;
            app.manage(AppState {
                settings: Mutex::new(settings::load(&settings_path)),
                settings_path,
                cache_dir,
                worker: Mutex::new(None),
                data_dir: app.path().data_dir()?,
                mic: Mic::spawn(),
                whisper: Arc::new(Whisper::default()),
                watcher: Mutex::new(None),
            });
            restart_watcher(app.handle(), &app.state::<AppState>());
            app.set_menu(app_menu(app.handle())?)?;
            app.on_menu_event(|app, event| {
                let id = event.id().as_ref();
                match id {
                    "undo" | "redo" | "help" | "focus" => {
                        // The page knows what's open, so it picks the help topic.
                        let _ = app.emit("menu", id);
                    }
                    "help:blog" => {
                        let _ = open_site("blog");
                    }
                    "help:issues" => {
                        let _ = open_site("issues");
                    }
                    _ => {}
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_notes,
            search_vault,
            read_note,
            write_note,
            create_note,
            import_image,
            save_image,
            read_image,
            list_manuscripts,
            load_manuscript,
            write_take,
            add_take,
            delete_take,
            restore_take,
            set_live,
            rename_take,
            set_block_title,
            set_block_props,
            set_target,
            set_order,
            add_block,
            delete_block,
            restore_block,
            cut_op,
            restore_cut,
            open_help,
            list_comments,
            set_comments,
            open_demo,
            new_book_folder,
            set_block_out,
            import_drafts,
            diff_words,
            voices,
            setup_narration,
            synth,
            get_lexicon,
            set_lexicon,
            dictate_start,
            dictate_stop,
            dictate_cancel,
            history_status,
            history_enable,
            history_snapshot,
            history_log,
            note_history,
            take_history,
            history_restore_all,
            export_book,
            export_audio_chapter,
            world_graph,
            world_timeline,
            world_tellings,
            world_entities,
            world_entity,
            world_scene,
            world_add,
            world_suggest,
            world_suggest_apply,
            world_import_csv,
            world_insert_event,
            world_move_event,
            world_delete_event,
            delete_note,
            create_manuscript,
            delete_manuscript,
            restore_manuscript,
            restore_note,
            library_report,
            library_add,
            library_remove,
            library_set_weights,
            cadence,
            script_add,
            script_remove,
            scripts_report,
            adaptation,
            script_open,
            get_cast,
            set_cast,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WRITEpad");
}

#[cfg(test)]
mod tests {
    use super::site_url;

    #[test]
    fn help_links_stay_on_the_site() {
        assert_eq!(site_url("cuts").unwrap(), "https://writepad-app.netlify.app/docs/cuts/");
        assert_eq!(site_url("").unwrap(), "https://writepad-app.netlify.app/docs/");
        assert_eq!(site_url("blog").unwrap(), "https://writepad-app.netlify.app/blog/");
        assert_eq!(site_url("python").unwrap(), "https://www.python.org/downloads/");
        for bad in ["../x", "https://evil.example", "cuts/../../", "Cuts", "a b", "x?y"] {
            assert_eq!(site_url(bad), None, "{bad}");
        }
    }
}
