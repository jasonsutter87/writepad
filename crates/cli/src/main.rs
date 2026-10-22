//! `writepad`: headless WRITEpad.
//!
//! ```text
//! writepad mcp                     MCP server on stdio, for agents
//! writepad tools                   list the tools
//! writepad call <tool> ['<json>']  run one tool, print JSON
//! writepad list                    manuscripts in the vault
//! writepad outline <manuscript>    blocks and takes
//!
//! --vault <dir>   (or WRITEPAD_VAULT; default: the vault open in the app)
//! --agent <name>  (or WRITEPAD_AGENT; default author for new takes)
//! ```

mod mcp;
mod tools;

use serde_json::{json, Value};
use std::path::PathBuf;
use tools::Ctx;

/// The vault the desktop app has open, from its settings file.
fn app_vault() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok()?);
    let mut candidates = vec![
        home.join("Library/Application Support/com.jasonsutter.writepad/settings.json"),
        home.join(".config/com.jasonsutter.writepad/settings.json"),
    ];
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("com.jasonsutter.writepad").join("settings.json"));
    }
    candidates.iter().find_map(|p| {
        let s: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
        s["vault"].as_str().map(PathBuf::from)
    })
}

fn usage() -> ! {
    eprintln!(
        "usage: writepad [--vault DIR] [--agent NAME] <mcp | tools | call TOOL [JSON] | list | outline MANUSCRIPT>"
    );
    std::process::exit(2)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        args.remove(i);
        (i < args.len()).then(|| args.remove(i))
    };
    let vault = flag("--vault").map(PathBuf::from).or_else(|| std::env::var("WRITEPAD_VAULT").ok().map(PathBuf::from)).or_else(app_vault);
    let agent = flag("--agent").or_else(|| std::env::var("WRITEPAD_AGENT").ok()).unwrap_or_else(|| "agent".into());
    let Some(command) = args.first().cloned() else { usage() };

    let Some(vault) = vault.filter(|v| v.is_dir()) else {
        eprintln!("No vault. Pass --vault DIR, set WRITEPAD_VAULT, or open one in the WRITEpad app.");
        std::process::exit(1)
    };
    let ctx = Ctx { vault, agent };

    let call = |name: &str, input: Value| {
        let tool = tools::all().into_iter().find(|t| t.name == name).unwrap_or_else(|| {
            eprintln!("No tool {name:?}. Run `writepad tools`.");
            std::process::exit(2)
        });
        match (tool.run)(&ctx, &input) {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        }
    };

    match command.as_str() {
        "mcp" => mcp::serve(&ctx),
        "tools" => {
            for t in tools::all() {
                println!("{:<18} {}", t.name, t.description);
            }
        }
        "call" => {
            let name = args.get(1).unwrap_or_else(|| usage());
            let input = match args.get(2) {
                Some(j) => serde_json::from_str(j).unwrap_or_else(|e| {
                    eprintln!("Bad JSON: {e}");
                    std::process::exit(2)
                }),
                None => json!({}),
            };
            call(name, input)
        }
        "list" => call("list_manuscripts", json!({})),
        "outline" => call("outline", json!({ "manuscript": args.get(1).unwrap_or_else(|| usage()) })),
        _ => usage(),
    }
}
