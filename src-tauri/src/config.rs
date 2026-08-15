use serde_json::Value;
use std::path::PathBuf;

pub const BASE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

pub fn base_dir() -> PathBuf {
    PathBuf::from(BASE_DIR)
}

pub fn config_dir() -> PathBuf {
    base_dir().join("config")
}

pub fn api_keys_path() -> PathBuf {
    config_dir().join("api_keys.json")
}

pub fn app_settings_path() -> PathBuf {
    config_dir().join("app_settings.json")
}

pub fn prompt_path() -> PathBuf {
    base_dir().join("core").join("prompt.txt")
}

pub fn rust_prompt_path() -> PathBuf {
    base_dir().join("core").join("prompt_rust.txt")
}

pub fn memory_path() -> PathBuf {
    base_dir().join("memory").join("long_term.json")
}

pub fn workspace_store_path() -> PathBuf {
    config_dir().join("workspace_store.sqlite3")
}

fn load_json(path: &PathBuf) -> Value {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

pub fn get_config() -> Value {
    load_json(&api_keys_path())
}

pub fn get_app_settings() -> Value {
    load_json(&app_settings_path())
}

pub fn get_api_key(name: &str) -> String {
    get_config()
        .get(name)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn has_api_key(name: &str) -> bool {
    !get_api_key(name).is_empty()
}

/// Returns a masked version of a stored key (first 4 chars + "••••") so the
/// frontend can show that a key is set without leaking its full value.
pub fn get_api_key_safe(name: &str) -> String {
    let k = get_api_key(name);
    if k.is_empty() {
        return String::new();
    }
    let visible: String = k.chars().take(4).collect();
    format!("{visible}••••••••")
}

pub fn gemini_api_key() -> String {
    get_api_key("gemini_api_key")
}

pub fn openrouter_api_key() -> String {
    get_api_key("openrouter_api_key")
}

pub fn anthropic_api_key() -> String {
    get_api_key("anthropic_api_key")
}

pub fn get_os() -> String {
    get_config()
        .get("os_system")
        .and_then(|v| v.as_str())
        .unwrap_or("windows")
        .to_lowercase()
}

pub fn is_windows() -> bool {
    get_os() == "windows"
}

pub fn is_mac() -> bool {
    get_os() == "mac"
}

pub fn is_linux() -> bool {
    get_os() == "linux"
}

pub fn developer_mode_enabled() -> bool {
    get_app_settings()
        .get("developer_mode_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ── AI provider settings (app_settings.json) ─────────────────────────────

pub fn get_ai_provider() -> String {
    get_app_settings()
        .get("ai_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("ollama")
        .trim()
        .to_lowercase()
}

pub fn set_ai_provider(provider: &str) -> Result<(), String> {
    set_app_setting("ai_provider", serde_json::Value::String(provider.trim().to_lowercase()))
}

pub fn get_ai_model(provider: &crate::ai::Provider) -> String {
    if *provider == crate::ai::Provider::AirLlm {
        return get_airllm_model();
    }
    let stored = get_app_settings()
        .get("ai_model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if stored.is_empty() {
        provider.default_model().to_string()
    } else {
        stored
    }
}

pub fn set_ai_model(model: &str) -> Result<(), String> {
    set_app_setting("ai_model", serde_json::Value::String(model.trim().to_string()))
}

pub fn get_airllm_model() -> String {
    let stored = get_app_settings()
        .get("airllm_model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if stored.is_empty() {
        crate::ai::Provider::AirLlm.default_model().to_string()
    } else {
        stored
    }
}

pub fn set_airllm_model(model: &str) -> Result<(), String> {
    set_app_setting("airllm_model", serde_json::Value::String(model.trim().to_string()))
}

pub fn get_ollama_host() -> String {
    let host = get_app_settings()
        .get("ollama_host")
        .and_then(|v| v.as_str())
        .unwrap_or("http://localhost:11434")
        .trim()
        .trim_end_matches('/')
        .to_string();
    if host.is_empty() {
        "http://localhost:11434".to_string()
    } else {
        host
    }
}

pub fn set_ollama_host(host: &str) -> Result<(), String> {
    set_app_setting("ollama_host", serde_json::Value::String(host.trim().to_string()))
}

pub fn developer_mode_workspace() -> String {
    get_app_settings()
        .get("developer_mode_workspace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Saves (or removes) a single key in api_keys.json, preserving other fields.
/// If `value` is empty, the key is removed from the file.
pub fn set_api_key(name: &str, value: &str) -> Result<(), String> {
    let path = api_keys_path();
    let mut cfg = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    if let Some(obj) = cfg.as_object_mut() {
        if value.is_empty() {
            obj.remove(name);
        } else {
            obj.insert(name.to_string(), serde_json::Value::String(value.trim().to_string()));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let pretty = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_app_setting(key: &str, value: Value) -> Result<(), String> {
    let path = app_settings_path();
    let mut cfg = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    if let Some(obj) = cfg.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let pretty = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_system_prompt() -> String {
    match std::fs::read_to_string(rust_prompt_path()) {
        Ok(text) => text,
        Err(_) => match std::fs::read_to_string(prompt_path()) {
            Ok(text) => text,
            Err(_) => "You are Ranveer, a sharp and efficient AI assistant. Stay calm, direct, and professional. Prefer tool use over guessing, and keep responses concise.\n".to_string(),
        },
    }
}