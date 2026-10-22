mod dictate;
mod settings;
mod watch;

use dictate::{Dictation, Mic, Whisper};
use manuscript::{Manuscript, ManuscriptInfo};
use narrate::{Lexicon, Worker};
use writepad_core::{diff, export, history, manuscript, narrate, profile, script, vault, world};
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
fn set_block_title(state: State<AppState>, manuscript: String, block: String, title: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_block_title(v, &manuscript, &block, &title))
}

#[tauri::command]
fn set_order(state: State<AppState>, manuscript: String, order: Vec<String>) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::set_order(v, &manuscript, &order))
}

#[tauri::command]
fn delete_block(state: State<AppState>, manuscript: String, block: String) -> Result<Manuscript, String> {
    changed(&state, &manuscript, |v| manuscript::delete_block(v, &manuscript, &block))
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

#[tauri::command]
async fn world_timeline(app: AppHandle) -> Result<Vec<world::TimelineEvent>, String> {
    blocking(app, |st| Ok(world::timeline(&st.vault()?))).await
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
async fn cadence(app: AppHandle, text: String) -> Result<profile::Cadence, String> {
    blocking(app, move |st| Ok(profile::cadence(&text, profile::target(&st.vault()?).as_ref()))).await
}

#[tauri::command]
async fn world_scene(app: AppHandle, manuscript: String, block: String) -> Result<world::SceneInfo, String> {
    blocking(app, move |st| world::scene_info(&st.vault()?, &manuscript, &block)).await
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
) -> Result<String, String> {
    let vault = state.vault()?;
    let mut props = vec![];
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

#[tauri::command]
fn restore_note(state: State<AppState>, id: String) -> Result<String, String> {
    world::restore_note(&state.vault()?, &id)
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
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::maximize(app, None)?, &PredefinedMenuItem::close_window(app, None)?],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
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
                if id == "undo" || id == "redo" {
                    let _ = app.emit("menu", id);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_notes,
            read_note,
            write_note,
            create_note,
            list_manuscripts,
            load_manuscript,
            write_take,
            add_take,
            delete_take,
            restore_take,
            set_live,
            rename_take,
            set_block_title,
            set_order,
            delete_block,
            cut_op,
            import_drafts,
            diff_words,
            voices,
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
            world_entities,
            world_entity,
            world_scene,
            world_add,
            world_insert_event,
            world_move_event,
            world_delete_event,
            delete_note,
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
