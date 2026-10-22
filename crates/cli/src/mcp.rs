//! A Model Context Protocol server over stdio: newline-delimited JSON-RPC 2.0.
//! Each agent session runs its own `writepad mcp` process; they coordinate
//! through the vault's file locks, not through each other.

use crate::tools::{self, Ctx};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

fn reply(out: &mut impl Write, id: &Value, result: Result<Value, (i64, String)>) {
    let msg = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    };
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

fn handle(ctx: &Ctx, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let version = if VERSIONS.contains(&asked) { asked } else { VERSIONS[0] };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "writepad", "version": env!("CARGO_PKG_VERSION") },
                "instructions": tools::INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": tools::all().iter().map(|t| json!({
                "name": t.name, "description": t.description, "inputSchema": (t.schema)(),
            })).collect::<Vec<_>>()
        })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let tool = tools::all().into_iter().find(|t| t.name == name).ok_or((-32602, format!("Unknown tool {name:?}")))?;
            // Tool failures are results the agent can read and act on, not protocol errors.
            Ok(match (tool.run)(ctx, &args) {
                Ok(v) => {
                    let text = match &v {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string_pretty(other).unwrap_or_default(),
                    };
                    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
                }
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ => Err((-32601, format!("Method not found: {method}"))),
    }
}

pub fn serve(ctx: &Ctx) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                reply(&mut out, &Value::Null, Err((-32700, format!("Parse error: {e}"))));
                continue;
            }
        };
        let method = msg["method"].as_str().unwrap_or("");
        match msg.get("id") {
            // Notifications (initialized, cancelled, ...) need no answer.
            None => continue,
            Some(id) => {
                let result = handle(ctx, method, msg.get("params").unwrap_or(&Value::Null));
                reply(&mut out, id, result);
            }
        }
    }
}
