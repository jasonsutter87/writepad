//! Local narration: a long-lived Kokoro worker process plus a sentence cache.
//!
//! The worker (tts/kokoro_worker.py, embedded at build time) loads the model
//! once and answers one JSON request per line on stdin. Rendered sentences
//! are cached as WAV files keyed by text + voice + speed + lexicon, so after
//! an edit only the sentences that changed are rendered again.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const WORKER_SCRIPT: &str = include_str!("../../../tts/kokoro_worker.py");

pub type Lexicon = BTreeMap<String, String>;

pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pub voices: Vec<String>,
}

impl Worker {
    pub fn start(python: &str, model_dir: &str, cache_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
        let script = cache_dir.join("kokoro_worker.py");
        fs::write(&script, WORKER_SCRIPT).map_err(|e| e.to_string())?;

        let mut child = Command::new(python)
            .arg(&script)
            .arg(model_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Couldn't start Python ({python}): {e}. Check Settings."))?;
        let stdin = child.stdin.take().ok_or("worker has no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("worker has no stdout")?);
        let mut worker = Worker { child, stdin, stdout, voices: vec![] };

        let hello = worker.read()?;
        if let Some(err) = hello.get("error").and_then(Value::as_str) {
            return Err(err.to_string());
        }
        worker.voices = hello["voices"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Ok(worker)
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn read(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("The narration worker exited. Is kokoro-onnx installed for that Python?".into());
        }
        serde_json::from_str(&line).map_err(|e| format!("Bad worker reply: {e}: {line}"))
    }

    fn request(&mut self, req: &Value) -> Result<Value, String> {
        writeln!(self.stdin, "{req}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let reply = self.read()?;
        match reply.get("error").and_then(Value::as_str) {
            Some(err) => Err(err.to_string()),
            None => Ok(reply),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn cache_path(cache_dir: &Path, text: &str, voice: &str, speed: f32, lexicon: &Lexicon) -> PathBuf {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    h.update([0]);
    h.update(voice.as_bytes());
    h.update(format!("\0{speed:.2}\0").as_bytes());
    h.update(serde_json::to_string(lexicon).unwrap_or_default().as_bytes());
    let key: String = h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
    cache_dir.join("tts").join(format!("{key}.wav"))
}

/// Render one sentence to WAV bytes, from cache when possible.
pub fn synth(
    worker: &mut Worker,
    cache_dir: &Path,
    text: &str,
    voice: &str,
    speed: f32,
    lexicon: &Lexicon,
) -> Result<Vec<u8>, String> {
    let out = cache_path(cache_dir, text, voice, speed, lexicon);
    if let Ok(bytes) = fs::read(&out) {
        return Ok(bytes);
    }
    fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    worker.request(&json!({
        "text": text,
        "voice": voice,
        "speed": speed,
        "lexicon": lexicon,
        "out": out,
    }))?;
    fs::read(&out).map_err(|e| e.to_string())
}

pub fn lexicon_path(vault: &Path) -> PathBuf {
    vault.join(".writepad").join("lexicon.json")
}

pub fn load_lexicon(vault: Option<&Path>) -> Lexicon {
    vault
        .and_then(|v| fs::read_to_string(lexicon_path(v)).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
