use crate::config;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// Ranveer Connect device registry + pairing, ported to Rust.
/// The Python version also runs a FastAPI/uvicorn HTTP+WebSocket gateway and
/// mDNS discovery. The Rust core keeps the same service surface and JSON device
/// registry so a native HTTP/WS gateway can be layered on later.

pub struct RanveerConnectService {
    registry_path: std::path::PathBuf,
    pub devices: Mutex<HashMap<String, Value>>,
}

impl RanveerConnectService {
    pub fn new(base_dir: &std::path::Path) -> Self {
        let registry_path = base_dir.join("config").join("ranveer_connect").join("devices.json");
        let devices = load_registry(&registry_path);
        RanveerConnectService {
            registry_path,
            devices: Mutex::new(devices),
        }
    }

    fn save(&self) {
        let _guard = self.devices.lock().unwrap();
        if let Some(parent) = self.registry_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = json!({ "devices": Value::Object(
            serde_json::Map::from_iter(
                _guard.iter().map(|(k, v)| (k.clone(), v.clone()))
            )
        )});
        if let Ok(text) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::write(&self.registry_path, text);
        }
    }

    pub fn list_devices(&self) -> Value {
        let guard = self.devices.lock().unwrap();
        let mut list: Vec<Value> = guard.values().cloned().collect();
        list.sort_by(|a, b| {
            let a_online = a.get("online").and_then(|v| v.as_bool()).unwrap_or(false);
            let b_online = b.get("online").and_then(|v| v.as_bool()).unwrap_or(false);
            b_online.cmp(&a_online)
        });
        json!({ "devices": list })
    }

    pub fn get_device(&self, query: &str) -> Option<Value> {
        let guard = self.devices.lock().unwrap();
        guard
            .iter()
            .find(|(id, _)| *id == query)
            .map(|(_, v)| v.clone())
            .or_else(|| {
                let q = query.to_lowercase();
                guard
                    .iter()
                    .find(|(_, v)| {
                        v.get("name")
                            .and_then(|n| n.as_str())
                            .map(|n| n.to_lowercase().contains(&q))
                            .unwrap_or(false)
                    })
                    .map(|(_, v)| v.clone())
            })
    }

    pub fn create_pairing_offer(&self) -> Value {
        use rand::Rng;
        let code = format!("{:06}", rand::thread_rng().gen_range(0..1000000));
        let token: String = (0..32)
            .map(|_| {
                let c = rand::thread_rng().gen_range(b'A'..=b'Z');
                c as char
            })
            .collect();
        let now = Utc::now().timestamp();
        json!({
            "pairing_code": code,
            "token": token,
            "expires_at": now + 300,
        })
    }

    pub fn pair_device(&self, device: Value) -> Value {
        let device_id = device
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if device_id.is_empty() {
            return json!({"success": false, "error": "device_id required"});
        }
        let mut record = device.clone();
        record["paired_at"] = json!(Utc::now().to_rfc3339());
        record["online"] = json!(true);
        record["last_seen"] = json!(Utc::now().timestamp());
        {
            let mut guard = self.devices.lock().unwrap();
            guard.insert(device_id.clone(), record);
        }
        self.save();
        json!({"success": true, "device": self.get_device(&device_id)})
    }

    pub fn disconnect_device(&self, device_id: &str) -> Value {
        {
            let mut guard = self.devices.lock().unwrap();
            if let Some(dev) = guard.get_mut(device_id) {
                dev["online"] = json!(false);
            }
        }
        self.save();
        json!({"success": true})
    }

    pub fn revoke_device(&self, device_id: &str) -> Value {
        {
            let mut guard = self.devices.lock().unwrap();
            guard.remove(device_id);
        }
        self.save();
        json!({"success": true})
    }

    pub fn gateway_info(&self) -> Value {
        json!({
            "service": "Ranveer Connect Gateway",
            "host": "0.0.0.0",
            "port": 8765,
            "enabled": true,
            "protocol_version": "1.0",
            "rust": true,
        })
    }

    pub fn route_command(&self, _target: &str, action: &str, parameters: &Value) -> Value {
        json!({
            "success": false,
            "action": action,
            "error": "Remote device command routing requires the native HTTP/WebSocket gateway (not yet enabled in this build).",
            "parameters": parameters,
        })
    }
}

fn load_registry(path: &std::path::Path) -> HashMap<String, Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            if let Ok(data) = serde_json::from_str::<Value>(&text) {
                if let Some(devices) = data.get("devices").and_then(|d| d.as_object()) {
                    return devices
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                }
            }
            HashMap::new()
        }
        Err(_) => HashMap::new(),
    }
}

pub fn get_service() -> &'static RanveerConnectService {
    static SERVICE: once_cell::sync::Lazy<RanveerConnectService> =
        once_cell::sync::Lazy::new(|| {
            let base = config::base_dir();
            RanveerConnectService::new(&base)
        });
    &SERVICE
}