//! App-level settings, stored as JSON in the OS config dir.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Folder holding notes and the `.writepad/` sidecar. Obsidian-compatible.
    pub vault: Option<String>,
    /// Python interpreter with `kokoro-onnx` installed.
    pub python: String,
    /// Folder containing kokoro-v1.0.onnx and voices-v1.0.bin.
    pub model_dir: String,
    pub voice: String,
    /// Voice for lines wholly in *italics* (thoughts, the link's voice).
    pub italic_voice: String,
    pub speed: f32,
    /// Seconds of silence after a paragraph.
    pub paragraph_pause: f32,
    /// Whisper model name, shared with murmur ("small", "base.en", ...).
    pub dictation_model: String,
    /// Whisper language code, or "auto".
    pub dictation_language: String,
    /// Tidy dictation with Claude (keeps your words, fixes mechanics).
    pub dictation_polish: bool,
    /// Snapshot the vault every 10 minutes while writing.
    pub history_autosave: bool,
    /// Byline for exports.
    pub author: String,
    /// "system" (follow the OS), "light", "dark" or "custom".
    pub theme: String,
    /// Colors for the custom theme.
    pub custom_theme: CustomTheme,
}

/// A custom color theme, like Slack's: a light or dark base plus five colors.
/// Muted text, rules and the desk are mixed from these in the UI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CustomTheme {
    /// "light" or "dark": which built-in palette the other colors start from.
    pub base: String,
    pub sidebar: String,
    pub background: String,
    pub page: String,
    pub text: String,
    pub accent: String,
}

impl Default for CustomTheme {
    fn default() -> Self {
        CustomTheme {
            base: "light".into(),
            sidebar: "#efe9de".into(),
            background: "#f6f2ea".into(),
            page: "#fffdf8".into(),
            text: "#23212b".into(),
            accent: "#d9774a".into(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        // Reuse the Kokoro setup from ChooseYourAdventure when it's there.
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
        let cya = Path::new(&home).join("Documents").join("ChooseYourAdventure");
        let venv_python = if cfg!(windows) { cya.join(".venv-tts\\Scripts\\python.exe") } else { cya.join(".venv-tts/bin/python") };
        Settings {
            vault: None,
            python: if venv_python.exists() {
                venv_python.to_string_lossy().into_owned()
            } else {
                if cfg!(windows) { "python".into() } else { "python3".into() }
            },
            model_dir: cya.join("tts_models").to_string_lossy().into_owned(),
            voice: "af_heart".into(),
            italic_voice: "af_nicole".into(),
            speed: 1.0,
            paragraph_pause: 0.55,
            dictation_model: "small".into(),
            dictation_language: "en".into(),
            dictation_polish: false,
            history_autosave: true,
            author: String::new(),
            theme: "system".into(),
            custom_theme: CustomTheme::default(),
        }
    }
}

pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &PathBuf, s: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}
