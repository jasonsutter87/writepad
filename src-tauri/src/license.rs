//! The Publish unlock: a Lemon Squeezy license key, activated online once
//! when it's entered, then remembered in license.json so EPUB and MP3 export
//! work offline from then on.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::Duration;

const API: &str = "https://api.lemonsqueezy.com/v1/licenses";
/// The WRITEpad store. Keys from any other Lemon Squeezy store don't unlock.
const STORE_ID: u64 = 508244;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct License {
    pub key: String,
    /// This computer's activation, so it can be released again.
    pub instance: String,
    pub email: String,
    pub name: String,
}

pub fn load(path: &Path) -> Option<License> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        // Lemon Squeezy explains a bad key in the body of a 4xx.
        .http_status_as_error(false)
        .build()
        .into()
}

fn post(endpoint: &str, form: &[(&str, &str)]) -> Result<Value, String> {
    agent()
        .post(format!("{API}/{endpoint}"))
        .header("Accept", "application/json")
        .send_form(form.iter().copied())
        .map_err(|e| format!("Couldn't reach Lemon Squeezy to check the key ({e}). Are you online?"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("Lemon Squeezy's answer: {e}"))
}

/// Activate `key` for this computer and remember it.
pub fn activate(path: &Path, key: &str) -> Result<License, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste the license key from your receipt email.".into());
    }
    let machine = format!("WRITEpad on {}", std::env::consts::OS);
    let license = parse_activation(key, &post("activate", &[("license_key", key), ("instance_name", &machine)])?)?;
    let json = serde_json::to_string_pretty(&license).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(license)
}

fn parse_activation(key: &str, resp: &Value) -> Result<License, String> {
    if resp["activated"] != true {
        let why = resp["error"].as_str().unwrap_or("Lemon Squeezy didn't accept it");
        return Err(format!("That key didn't unlock Publish: {why}"));
    }
    if resp["meta"]["store_id"].as_u64() != Some(STORE_ID) {
        return Err("That key is for a different product, not WRITEpad Publish.".into());
    }
    Ok(License {
        key: key.to_string(),
        instance: resp["instance"]["id"].as_str().unwrap_or_default().to_string(),
        email: resp["meta"]["customer_email"].as_str().unwrap_or_default().to_string(),
        name: resp["meta"]["customer_name"].as_str().unwrap_or_default().to_string(),
    })
}

/// Forget the key here and free this computer's activation for another one.
/// Forgetting works offline; the activation is released when Lemon Squeezy
/// can be reached.
pub fn deactivate(path: &Path) -> Result<(), String> {
    if let Some(l) = load(path) {
        if !l.instance.is_empty() {
            let _ = post("deactivate", &[("license_key", &l.key), ("instance_id", &l.instance)]);
        }
    }
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn activated(store: u64) -> Value {
        json!({
            "activated": true, "error": null,
            "license_key": { "status": "active", "activation_limit": 5, "activation_usage": 1 },
            "instance": { "id": "47596ad9-a811-4ebf-ac8a-03fc7b6d2a17", "name": "WRITEpad on macos" },
            "meta": { "store_id": store, "product_id": 1476247, "customer_name": "Ada", "customer_email": "ada@example.com" }
        })
    }

    #[test]
    fn accepts_a_key_from_our_store() {
        let l = parse_activation("KEY", &activated(STORE_ID)).unwrap();
        assert_eq!(l.instance, "47596ad9-a811-4ebf-ac8a-03fc7b6d2a17");
        assert_eq!(l.email, "ada@example.com");
    }

    #[test]
    fn refuses_other_stores_and_lemon_squeezy_errors() {
        assert!(parse_activation("KEY", &activated(1)).unwrap_err().contains("different product"));
        let err = json!({ "activated": false, "error": "license_key not found." });
        assert!(parse_activation("KEY", &err).unwrap_err().contains("license_key not found"));
    }

    #[test]
    fn remembers_and_forgets_offline() {
        let dir = std::env::temp_dir().join(format!("wp-license-{}", std::process::id()));
        let path = dir.join("license.json");
        let l = parse_activation("KEY", &activated(STORE_ID)).unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, serde_json::to_string(&License { instance: String::new(), ..l.clone() }).unwrap()).unwrap();
        assert_eq!(load(&path).unwrap().email, "ada@example.com");
        deactivate(&path).unwrap();
        assert!(load(&path).is_none());
        deactivate(&path).unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
