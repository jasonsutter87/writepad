//! One-click narration setup: a private Python venv with kokoro-onnx, plus the
//! two Kokoro model files, under the app's data dir. Needs a Python 3.10+ on
//! the machine to build the venv from; everything else it fetches itself.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MODELS: [(&str, &str); 2] = [
    ("kokoro-v1.0.onnx", "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx"),
    ("voices-v1.0.bin", "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin"),
];

#[derive(Serialize, Clone)]
pub struct Progress {
    pub message: String,
}

pub struct Installed {
    pub python: String,
    pub model_dir: String,
}

pub fn venv_python(root: &Path) -> PathBuf {
    if cfg!(windows) { root.join(".venv").join("Scripts").join("python.exe") } else { root.join(".venv").join("bin").join("python") }
}

/// Build (or repair) the venv and fetch the models into `root`. Safe to re-run:
/// finished steps are skipped.
pub fn install(root: &Path, preferred: &str, progress: impl Fn(String)) -> Result<Installed, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let python = venv_python(root);

    if !python.exists() {
        progress("Looking for Python…".into());
        let base = find_python(preferred).ok_or(
            "Narration needs Python 3.10 or newer, and none was found. Install it from python.org, then try again.",
        )?;
        progress("Creating a Python environment…".into());
        check(run(&base.0, &[&base.1[..], &["-m".into(), "venv".into(), root.join(".venv").to_string_lossy().into()]].concat()), "Creating the venv")?;
    }

    if run(&python.to_string_lossy(), &args(&["-c", "import kokoro_onnx, soundfile"])).map(|o| o.status.success()) != Ok(true) {
        progress("Installing Kokoro (a minute or two)…".into());
        check(
            run(&python.to_string_lossy(), &args(&["-m", "pip", "install", "--disable-pip-version-check", "kokoro-onnx", "soundfile"])),
            "Installing kokoro-onnx",
        )?;
    }

    let model_dir = root.join("models");
    fs::create_dir_all(&model_dir).map_err(|e| e.to_string())?;
    for (name, url) in MODELS {
        let dest = model_dir.join(name);
        if dest.exists() {
            continue;
        }
        murmur_core::download::fetch(url, &dest, |done, total| {
            let mb = |b: u64| b / (1 << 20);
            progress(if total > 0 { format!("Downloading {name}: {} of {} MB", mb(done), mb(total)) } else { format!("Downloading {name}: {} MB", mb(done)) });
        })
        .map_err(|e| format!("Downloading {name}: {e}"))?;
    }

    Ok(Installed { python: python.to_string_lossy().into_owned(), model_dir: model_dir.to_string_lossy().into_owned() })
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// The first interpreter that runs and is new enough, as (program, leading args).
/// Windows' Store alias `python` fails the check, so it's skipped.
fn find_python(preferred: &str) -> Option<(String, Vec<String>)> {
    let mut candidates: Vec<(String, Vec<String>)> = vec![];
    if !preferred.is_empty() {
        candidates.push((preferred.into(), vec![]));
    }
    if cfg!(windows) {
        candidates.push(("py".into(), args(&["-3"])));
    }
    candidates.push(("python3".into(), vec![]));
    candidates.push(("python".into(), vec![]));
    let probe = "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)";
    candidates.into_iter().find(|(prog, lead)| {
        run(prog, &[&lead[..], &args(&["-c", probe])].concat()).map(|o| o.status.success()) == Ok(true)
    })
}

fn run(program: &str, args: &[String]) -> Result<Output, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map_err(|e| format!("{program}: {e}"))
}

fn check(out: Result<Output, String>, what: &str) -> Result<(), String> {
    let out = out.map_err(|e| format!("{what}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = tail[tail.len().saturating_sub(3)..].join(" ");
    Err(format!("{what} failed: {tail}"))
}
