//! `cargo run -p writepad-devserver` → http://127.0.0.1:1421/api/<command>
//!
//! Development only: the app's commands over HTTP, so the UI runs in a normal
//! browser (`npm run dev`, then http://localhost:1420) and tools like
//! auto-documentor can click through it. Vite proxies `/api` here.
//!
//! The vault is a fresh copy of `demo/` in `target/docs-vault`, never a real
//! one, unless a folder is given on the command line. `POST /api/__reset`
//! copies the demo again. Narration, dictation and file dialogs need the
//! desktop app, so those commands say so.
//!
//! It answers only requests addressed to `127.0.0.1:1421` or `localhost:1421`
//! (so a page that rebinds its own domain to this machine is refused), from
//! no origin or the Vite dev origin, with a JSON content type.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use writepad_core::{diff, history, manuscript, narrate, profile, script, search, suggest, vault, world};

const ADDR: &str = "127.0.0.1:1421";
const HOSTS: [&str; 2] = ["127.0.0.1:1421", "localhost:1421"];
const ORIGINS: [&str; 2] = ["http://localhost:1420", "http://127.0.0.1:1420"];
const DESKTOP: &str = "Narration, dictation and files need the desktop app; this is the browser preview.";

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (vault, sandbox) = match std::env::args().nth(1) {
        Some(v) => (PathBuf::from(v), false),
        None => (root.join("target/docs-vault"), true),
    };
    let demo = root.join("demo");
    if sandbox {
        reset(&demo, &vault).expect("copy the demo vault");
    }
    let vault = vault.canonicalize().expect("vault folder");
    let mut settings = default_settings(&vault);
    let server = tiny_http::Server::http(ADDR).expect("bind 127.0.0.1:1421");
    eprintln!("writepad-devserver: {ADDR}, vault {}", vault.display());

    for mut req in server.incoming_requests() {
        let reply = |status: u16, body: Value| tiny_http::Response::from_string(body.to_string()).with_status_code(status).with_header(
            "Content-Type: application/json".parse::<tiny_http::Header>().unwrap(),
        );
        if let Err(status) = allowed(&req) {
            let _ = req.respond(reply(status, json!("refused")));
            continue;
        }
        let Some(cmd) = req.url().strip_prefix("/api/").map(str::to_string) else {
            let _ = req.respond(reply(404, json!("not found")));
            continue;
        };
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let args: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let result = match cmd.as_str() {
            "__reset" if sandbox => reset(&demo, &vault).map(|_| {
                settings = default_settings(&vault);
                Value::Null
            }),
            "__reset" => Err("Not resetting a vault given on the command line.".into()),
            "get_settings" => Ok(settings.clone()),
            "save_settings" => {
                // The preview stays on its vault whatever the page asks for.
                let mut s = args["settings"].clone();
                s["vault"] = json!(vault);
                settings = s;
                Ok(Value::Null)
            }
            _ => call(&vault, &cmd, &args),
        };
        let _ = match result {
            Ok(v) => req.respond(reply(200, v)),
            Err(e) => req.respond(reply(422, json!(e))),
        };
    }
}

/// Whether a request may reach the vault, judged by its headers.
fn allowed(req: &tiny_http::Request) -> Result<(), u16> {
    fn header<'a>(req: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
        req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
    }
    let header = |name: &'static str| header(req, name);
    if req.method() != &tiny_http::Method::Post || !header("Host").is_some_and(|h| HOSTS.contains(&h)) {
        return Err(403);
    }
    if header("Origin").is_some_and(|o| !ORIGINS.contains(&o)) {
        return Err(403);
    }
    if !header("Content-Type").is_some_and(|c| c.split(';').next().is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))) {
        return Err(415);
    }
    Ok(())
}

/// Replace the sandbox vault with a fresh copy of the demo.
fn reset(demo: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        fs::remove_dir_all(to).map_err(|e| e.to_string())?;
    }
    copy(demo, to).map_err(|e| e.to_string())
}

fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".lock") {
            continue;
        }
        if entry.file_type()?.is_dir() {
            copy(&entry.path(), &to.join(name))?;
        } else {
            fs::copy(entry.path(), to.join(name))?;
        }
    }
    Ok(())
}

/// The desktop app's defaults (src-tauri/src/settings.rs), on the sandbox vault.
fn default_settings(vault: &Path) -> Value {
    json!({
        "vault": vault, "python": "", "model_dir": "", "voice": "af_heart", "italic_voice": "af_nicole",
        "speed": 1.0, "paragraph_pause": 0.55, "dictation_model": "small", "dictation_language": "en",
        "dictation_polish": false, "history_autosave": false, "author": "", "theme": "light",
        "custom_theme": { "base": "light", "sidebar": "#efe9de", "background": "#f6f2ea", "page": "#fffdf8", "text": "#23212b", "accent": "#d9774a" },
    })
}

fn arg<T: serde::de::DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{name}: {e}"))
}

fn ok<T: serde::Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.map(|v| serde_json::to_value(v).unwrap())
}

/// One app command, the same work as its `#[tauri::command]` in src-tauri.
fn call(v: &Path, cmd: &str, a: &Value) -> Result<Value, String> {
    let s = |name: &str| arg::<String>(a, name);
    let o = |name: &str| arg::<Option<String>>(a, name);
    // Structural changes return the fresh manuscript, as in the app.
    let changed = |f: &dyn Fn(&str) -> Result<(), String>| -> Result<Value, String> {
        let id = s("manuscript")?;
        f(&id)?;
        ok(manuscript::load(v, &id))
    };
    match cmd {
        "list_notes" => ok(Ok(vault::list_notes(v))),
        "search_vault" => ok(Ok(search::search(v, &s("query")?, 200))),
        "read_note" => ok(vault::read(v, &s("path")?)),
        "write_note" => ok(vault::write(v, &s("path")?, &s("text")?)),
        "create_note" => ok(vault::create(v, &s("name")?)),
        "list_manuscripts" => ok(Ok(manuscript::list(v))),
        "load_manuscript" => ok(manuscript::load(v, &s("id")?)),
        "write_take" => ok(manuscript::write_take(v, &s("manuscript")?, &s("take")?, &s("text")?)),
        "add_take" => changed(&|m| manuscript::add_take(v, m, &s("block")?, o("after")?.as_deref(), &s("label")?, "", &s("text")?).map(|_| ())),
        "delete_take" => changed(&|m| manuscript::delete_take(v, m, &s("take")?)),
        "restore_take" => changed(&|m| manuscript::restore_take(v, m, &s("take")?)),
        "set_live" => changed(&|m| manuscript::set_live(v, m, &s("take")?)),
        "rename_take" => changed(&|m| manuscript::rename_take(v, m, &s("take")?, &s("label")?)),
        "set_block_props" => changed(&|m| manuscript::set_block_props(v, m, &s("block")?, arg(a, "props")?)),
        "set_block_out" => changed(&|m| manuscript::set_block_out(v, m, &s("block")?, arg(a, "out")?)),
        "restore_cut" => changed(&|m| manuscript::restore_cut(v, m, arg(a, "cut")?, arg(a, "at")?)),
        "set_target" => changed(&|m| manuscript::set_target(v, m, arg(a, "target")?)),
        "set_block_title" => changed(&|m| manuscript::set_block_title(v, m, &s("block")?, &s("title")?)),
        "set_order" => changed(&|m| manuscript::set_order(v, m, &arg::<Vec<String>>(a, "order")?)),
        "add_block" => changed(&|m| manuscript::add_block(v, m, &s("title")?, o("after")?.as_deref(), "", "").map(|_| ())),
        "delete_block" => changed(&|m| manuscript::delete_block(v, m, &s("block")?)),
        "restore_block" => changed(&|m| manuscript::restore_block(v, m, &s("block")?)),
        "cut_op" => changed(&|m| {
            let name = s("name")?;
            match s("op")?.as_str() {
                "save" => manuscript::save_cut(v, m, &name),
                "apply" => manuscript::apply_cut(v, m, &name),
                "delete" => manuscript::delete_cut(v, m, &name),
                op => Err(format!("Unknown cut op {op}")),
            }
        }),
        "diff_words" => ok(Ok(diff::words(&s("old")?, &s("new")?))),
        "voices" => ok(Ok(Vec::<String>::new())),
        "get_lexicon" => ok(Ok(narrate::load_lexicon(Some(v)))),
        "set_lexicon" => {
            let lexicon: narrate::Lexicon = arg(a, "lexicon")?;
            let path = narrate::lexicon_path(v);
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            ok(fs::write(path, serde_json::to_string_pretty(&lexicon).unwrap()).map_err(|e| e.to_string()))
        }
        "history_status" => ok(Ok(history::status(v))),
        "history_enable" => ok(history::enable(v)),
        "history_snapshot" => ok(history::snapshot(v, &s("message")?)),
        "history_log" => ok(history::log(v, 200)),
        "note_history" => ok(history::note_history(v, &s("path")?, 100)),
        "take_history" => ok(history::take_history(v, &s("manuscript")?, &s("take")?, 100)),
        "history_restore_all" => ok(history::restore_all(v, &s("id")?)),
        "world_graph" => ok(Ok(world::graph(v))),
        "world_timeline" => {
            let (m, cut) = (o("manuscript")?, o("cut")?);
            ok(world::timeline_read(v, m.as_deref().map(|m| (m, cut.as_deref()))))
        }
        "world_tellings" => ok(Ok(manuscript::list(v)
            .into_iter()
            .map(|m| {
                let cuts: Vec<String> = manuscript::manifest(v, &m.id).map(|x| x.cuts.into_iter().map(|c| c.name).collect()).unwrap_or_default();
                json!({ "id": m.id, "title": m.title, "cuts": cuts })
            })
            .collect::<Vec<_>>())),
        "world_entities" => ok(Ok(world::entities(v))),
        "world_entity" => ok(Ok(world::entity_info(v, &s("path")?).ok())),
        "world_scene" => ok(world::scene_info(v, &s("manuscript")?, &s("block")?)),
        "library_report" => ok(Ok(profile::report(v, o("manuscript")?.as_deref()))),
        "library_remove" => ok(profile::remove(v, &s("id")?)),
        "library_set_weights" => ok(profile::set_weights(v, &arg::<BTreeMap<String, f64>>(a, "weights")?)),
        "cadence" => {
            let mine = o("manuscript")?.and_then(|id| manuscript::manifest(v, &id).ok()).and_then(|m| m.target);
            ok(Ok(profile::cadence(&s("text")?, profile::target(v).as_ref(), mine.as_ref())))
        }
        "script_open" => ok(script::open_as_manuscript(v, &s("id")?)),
        "script_add" => ok(script::add(v, s("source")?.trim(), None, None).map(|x| script::summarize(&x))),
        "script_remove" => ok(script::remove(v, &s("id")?)),
        "scripts_report" => ok(Ok(script::report(v))),
        "adaptation" => ok(script::adapt(v, &s("book")?, &s("scriptId")?)),
        "get_cast" => {
            let all: BTreeMap<String, BTreeMap<String, String>> = fs::read_to_string(cast_path(v)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            ok(Ok(all.get(&s("scope")?).cloned().unwrap_or_default()))
        }
        "set_cast" => {
            let path = cast_path(v);
            let mut all: BTreeMap<String, Value> = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            all.insert(s("scope")?, a["cast"].clone());
            ok(writepad_core::write_atomic(&path, serde_json::to_string_pretty(&all).unwrap().as_bytes()))
        }
        "world_suggest" => ok(suggest::suggest(v, &s("manuscript")?)),
        "world_suggest_apply" => ok(suggest::apply(v, &s("manuscript")?, &arg::<Vec<suggest::Suggestion>>(a, "picks")?, arg(a, "timeline")?)),
        "world_add" => ok(world::add(v, &s("kind")?, &s("name")?, &[], "")),
        "world_insert_event" => {
            let mut props = vec![];
            if let Some(t) = o("timeline")?.filter(|t| !t.trim().is_empty()) {
                props.push(("timeline".to_string(), json!(t.trim()).to_string()));
            }
            if let Some(w) = o("when")?.filter(|w| !w.trim().is_empty()) {
                props.push(("when".to_string(), json!(w.trim()).to_string()));
            }
            if let Some((m, block)) = arg::<Option<(String, String)>>(a, "scene")? {
                let link = format!("[[{}]]", world::scene_ref(v, &m, &block)?);
                props.push(("scenes".to_string(), format!("[{}]", json!(link))));
            }
            ok(world::insert_event(v, &s("name")?, o("after")?.as_deref(), o("before")?.as_deref(), props, ""))
        }
        "world_move_event" => ok(world::move_event(v, &s("path")?, o("after")?.as_deref(), o("before")?.as_deref())),
        "world_delete_event" => ok(world::delete_event(v, &s("path")?)),
        "delete_note" => ok(world::delete_note(v, &s("path")?)),
        "restore_note" => ok(world::restore_note(v, &s("id")?)),
        "create_manuscript" => ok(manuscript::new_blank(v, &s("title")?, arg(a, "screenplay")?)),
        "delete_manuscript" => ok(manuscript::trash_manuscript(v, &s("id")?)),
        "restore_manuscript" => ok(manuscript::restore_manuscript(v, &s("id")?)),
        "export_book" | "export_audio_chapter" | "import_drafts" | "library_add" | "world_import_csv" | "setup_narration" | "synth"
        | "dictate_start" | "dictate_stop" | "dictate_cancel" => Err(DESKTOP.into()),
        _ => Err(format!("Unknown command {cmd}")),
    }
}

fn cast_path(v: &Path) -> PathBuf {
    v.join(".writepad").join("cast.json")
}
