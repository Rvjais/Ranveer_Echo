use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub key: String,
    pub name: String,
    pub available: bool,
    pub coming_soon: bool,
}

pub struct SmartHomeService {
    // provider credentials loaded from config/smart_home.json (plaintext fallback)
    // and device records. The Python version keeps per-provider cloud clients
    // (Kasa, Atomberg, Hue, LG, Daikin, Tuya, Nest, SmartThings) which each
    // require their own SDK; the Rust core exposes the same service surface.
}

impl SmartHomeService {
    pub fn new() -> Self {
        SmartHomeService {}
    }

    pub fn list_platforms(&self) -> Value {
        json!([
            {"key": "kasa", "name": "TP-Link Kasa", "available": false, "coming_soon": true},
            {"key": "atomberg", "name": "Atomberg", "available": false, "coming_soon": true},
            {"key": "hue", "name": "Philips Hue", "available": false, "coming_soon": true},
            {"key": "lg", "name": "LG ThinQ", "available": false, "coming_soon": true},
            {"key": "daikin", "name": "Daikin", "available": false, "coming_soon": true},
            {"key": "tuya", "name": "Tuya Smart", "available": false, "coming_soon": true},
            {"key": "nest", "name": "Google Nest", "available": false, "coming_soon": true},
            {"key": "smartthings", "name": "SmartThings", "available": false, "coming_soon": true},
        ])
    }

    pub fn list_devices(&self) -> Vec<Value> {
        let path = crate::config::config_dir().join("smart_home_devices.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Vec<Value>>(&text).unwrap_or_default(),
            Err(_) => vec![],
        }
    }

    pub fn execute_command(&self, command: &str) -> Value {
        let detail = format!(
            "Smart-home command handled locally, but cloud providers (Kasa, Atomberg, Hue, ...) are not connected in the Rust build yet. Command: {command}"
        );
        json!({
            "action": "control",
            "detail": detail,
            "success": false,
        })
    }
}

static SMART_HOME: Lazy<SmartHomeService> = Lazy::new(SmartHomeService::new);

pub fn service() -> &'static SmartHomeService {
    &SMART_HOME
}

pub fn platforms_map() -> HashMap<&'static str, PlatformInfo> {
    let mut map = HashMap::new();
    for p in [
        ("kasa", "TP-Link Kasa"),
        ("atomberg", "Atomberg"),
        ("hue", "Philips Hue"),
        ("lg", "LG ThinQ"),
        ("daikin", "Daikin"),
        ("tuya", "Tuya Smart"),
        ("nest", "Google Nest"),
        ("smartthings", "SmartThings"),
    ] {
        map.insert(
            p.0,
            PlatformInfo {
                key: p.0.to_string(),
                name: p.1.to_string(),
                available: false,
                coming_soon: true,
            },
        );
    }
    map
}