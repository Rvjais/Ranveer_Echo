//! Local LLM inference through Ollama.
//!
//! Host and model are configurable from Settings (defaults: localhost:11434,
//! dolphin-phi:2.7b). All other providers go through `crate::ai`.

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::Mutex;

pub const OLLAMA_URL: &str = "http://localhost:11434";
pub const LOCAL_MODEL: &str = "dolphin-phi:2.7b";

/// Cached model name so we resolve it once per session.
static MODEL_CACHE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Clears the cached model so the next call re-reads Settings. Called when the
/// model or host changes, otherwise chats silently keep using the old model.
pub fn invalidate_model_cache() {
    if let Ok(mut g) = MODEL_CACHE.lock() {
        *g = None;
    }
}

/// Base URL of the configured Ollama host (Settings > Ollama host).
pub fn base_url() -> String {
    crate::config::get_ollama_host()
}

/// The configured model name (Settings > model) for the Ollama provider.
pub fn configured_model() -> String {
    crate::config::get_ai_model(&crate::ai::Provider::Ollama)
}

/// Async, non-blocking model resolution — safe to call inside the tokio runtime.
/// Prefers the model chosen in Settings; falls back to the best installed one.
pub async fn resolve_model() -> String {
    if let Some(m) = MODEL_CACHE.lock().ok().and_then(|g| g.clone()) {
        return m;
    }
    let chosen = fetch_best_model().await.unwrap_or_else(|| configured_model());
    if let Ok(mut g) = MODEL_CACHE.lock() {
        *g = Some(chosen.clone());
    }
    chosen
}

async fn fetch_best_model() -> Option<String> {
    let configured = configured_model();
    let candidates = [
        "dolphin-phi:2.7b",
        "dolphin-mistral:7b",
        "qwen2.5:3b",
        "llama3.2:3b",
        "phi3:3.8b",
    ];
    let client = http_client().ok()?;
    let resp = client
        .get(format!("{}/api/tags", base_url()))
        .send()
        .await
        .ok()?;
    let data = resp.json::<Value>().await.ok()?;
    let names: Vec<String> = data["models"]
        .as_array()?
        .iter()
        .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
        .collect();
    if names.iter().any(|n| n == &configured) {
        return Some(configured);
    }
    if let Some(c) = candidates
        .iter()
        .find(|c| names.iter().any(|n| n.starts_with(**c)))
    {
        return Some(c.to_string());
    }
    names.into_iter().next()
}
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

/// Checks whether Ollama is reachable (works from async or sync contexts).
pub fn is_available() -> bool {
    is_running()
}

/// Simple availability probe using a blocking client.
pub fn is_running() -> bool {
    match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => match c.get(format!("{}/api/tags", base_url())).send() {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Picks the best local model: prefer the Settings model, else a lighter one.
pub fn local_model_name() -> String {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return configured_model(),
    };
    let configured = configured_model();
    let candidates = [
        "dolphin-phi:2.7b",
        "dolphin-mistral:7b",
        "qwen2.5:3b",
        "llama3.2:3b",
        "phi3:3.8b",
    ];
    let tags = client.get(format!("{}/api/tags", base_url())).send();
    let names: Vec<String> = match tags {
        Ok(r) => match r.json::<Value>() {
            Ok(data) => data["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => vec![],
        },
        Err(_) => vec![],
    };
    if names.iter().any(|n| n == &configured) {
        return configured;
    }
    for m in candidates {
        if names.iter().any(|n| n.starts_with(m)) {
            return m.to_string();
        }
    }
    configured
}

pub async fn chat(
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
) -> Result<String, String> {
    let client = http_client()?;
    let model = resolve_model().await;
    let system = system.unwrap_or("You are Ranveer, a concise, helpful desktop assistant. Be natural and brief.");
    let body = json!({
        "model": model,
        "prompt": format!("{system}\n\nUser: {prompt}"),
        "stream": false,
        "options": {
            "temperature": temperature,
            "num_predict": max_tokens,
            "num_ctx": 4096,
        }
    });
    let resp = client
        .post(format!("{}/api/generate", base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("[Ollama] request error: {e}"))?;
    let status = resp.status();
    let data: Value = resp.json().await.map_err(|e| format!("[Ollama] parse error: {e}"))?;
    if status != reqwest::StatusCode::OK {
        let err = data["error"].as_str().unwrap_or("unknown error");
        return Err(format!("[Ollama] HTTP {status}: {err}"));
    }
    let text = data["response"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Err("[Ollama] empty response.".to_string());
    }
    Ok(text)
}

/// Streaming `/api/generate` — token deltas forwarded to `on_text`.
pub async fn chat_stream(
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    use futures_util::StreamExt;
    let client = http_client()?;
    let model = resolve_model().await;
    let system = system.unwrap_or("You are Ranveer, a concise, helpful desktop assistant. Be natural and brief.");
    let body = json!({
        "model": model,
        "prompt": format!("{system}\n\nUser: {prompt}"),
        "stream": true,
        "options": {
            "temperature": temperature,
            "num_predict": max_tokens,
            "num_ctx": 4096,
        }
    });
    let resp = client
        .post(format!("{}/api/generate", base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("[Ollama] request error: {e}"))?;
    let status = resp.status();
    if status != reqwest::StatusCode::OK {
        let data: Value = resp.json().await.unwrap_or(Value::Null);
        let err = data["error"].as_str().unwrap_or("unknown error");
        return Err(format!("[Ollama] HTTP {status}: {err}"));
    }
    let mut full = String::new();
    let mut buffer = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("[Ollama] stream error: {e}"))?;
        buffer.extend_from_slice(&chunk);
        while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                if let Some(t) = v["response"].as_str() {
                    if !t.is_empty() {
                        full.push_str(t);
                        on_text(t);
                    }
                }
                if v["done"].as_bool().unwrap_or(false) {
                    return Ok(full.trim().to_string());
                }
            }
        }
    }
    let text = full.trim().to_string();
    if text.is_empty() {
        return Err("[Ollama] empty response.".to_string());
    }
    Ok(text)
}

/// Vision chat: sends a base64 image alongside the prompt to a vision-capable
/// local model (e.g. llama3.2-vision).
pub async fn chat_with_image(
    prompt: &str,
    system: Option<&str>,
    model: String,
    image_b64: &str,
    _mime: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let client = http_client()?;
    let system = system.unwrap_or("You are Ranveer, a helpful desktop assistant.");
    let body = json!({
        "model": model,
        "prompt": format!("{system}\n\nUser: {prompt}"),
        "images": [image_b64],
        "stream": false,
        "options": {
            "temperature": 0.4,
            "num_predict": max_tokens,
            "num_ctx": 8192,
        }
    });
    let resp = client
        .post(format!("{}/api/generate", base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("[Ollama] vision request error: {e}"))?;
    let status = resp.status();
    let data: Value = resp.json().await.map_err(|e| format!("[Ollama] parse error: {e}"))?;
    if status != reqwest::StatusCode::OK {
        let err = data["error"].as_str().unwrap_or("unknown error");
        return Err(format!("[Ollama] HTTP {status}: {err}"));
    }
    let text = data["response"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Err("[Ollama] empty vision response.".to_string());
    }
    Ok(text)
}

pub async fn chat_json(
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
) -> Result<Value, String> {
    crate::ai::chat_json(prompt, system, max_tokens).await
}