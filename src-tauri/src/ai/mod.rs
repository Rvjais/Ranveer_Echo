//! Multi-provider LLM abstraction.
//!
//! Every text/automation call in the app goes through this module, which reads
//! the active provider from `config/app_settings.json` and dispatches to:
//!   - Local (Ollama)  — any local model on a configurable host
//!   - OpenAI / DeepSeek / OpenRouter — OpenAI-compatible chat completions
//!   - Gemini — Google's generateContent API
//!   - Anthropic (Claude) — /v1/messages API
//!
//! API keys are read from `config/api_keys.json` (never logged, masked in UI).

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use base64::Engine;
use serde_json::{json, Value};
use std::sync::Mutex;

pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Ollama,
    OpenAI,
    DeepSeek,
    OpenRouter,
    Gemini,
    Anthropic,
    AirLlm,
}

impl Provider {
    pub fn from_str(s: &str) -> Provider {
        match s.trim().to_lowercase().as_str() {
            "openai" => Provider::OpenAI,
            "deepseek" => Provider::DeepSeek,
            "openrouter" => Provider::OpenRouter,
            "gemini" => Provider::Gemini,
            "anthropic" | "claude" => Provider::Anthropic,
            "airllm" => Provider::AirLlm,
            _ => Provider::Ollama,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Ollama => "ollama",
            Provider::OpenAI => "openai",
            Provider::DeepSeek => "deepseek",
            Provider::OpenRouter => "openrouter",
            Provider::Gemini => "gemini",
            Provider::Anthropic => "anthropic",
            Provider::AirLlm => "airllm",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Provider::Ollama => "Local (Ollama)",
            Provider::OpenAI => "OpenAI",
            Provider::DeepSeek => "DeepSeek",
            Provider::OpenRouter => "OpenRouter",
            Provider::Gemini => "Gemini",
            Provider::Anthropic => "Claude (Anthropic)",
            Provider::AirLlm => "Local (AirLLM)",
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Provider::Ollama => "dolphin-phi:2.7b",
            Provider::OpenAI => "gpt-4o-mini",
            Provider::DeepSeek => "deepseek-chat",
            Provider::OpenRouter => "openai/gpt-4o-mini",
            Provider::Gemini => "gemini-2.5-flash",
            Provider::Anthropic => "claude-3-5-haiku-latest",
            Provider::AirLlm => "Qwen/Qwen2.5-0.5B",
        }
    }

    fn key_name(&self) -> &'static str {
        match self {
            Provider::Ollama => "",
            Provider::OpenAI => "openai_api_key",
            Provider::DeepSeek => "deepseek_api_key",
            Provider::OpenRouter => "openrouter_api_key",
            Provider::Gemini => "gemini_api_key",
            Provider::Anthropic => "anthropic_api_key",
            Provider::AirLlm => "",
        }
    }

    fn needs_api_key(&self) -> bool {
        matches!(
            self,
            Provider::OpenAI
                | Provider::DeepSeek
                | Provider::OpenRouter
                | Provider::Gemini
                | Provider::Anthropic
        )
    }
}

/// The active AI configuration, resolved from app settings + stored keys.
#[derive(Debug, Clone)]
pub struct AiSettings {
    pub provider: Provider,
    pub model: String,
    pub ollama_host: String,
    pub api_key: String,
}

pub fn settings() -> AiSettings {
    let provider = Provider::from_str(&crate::config::get_ai_provider());
    let model = crate::config::get_ai_model(&provider);
    let host = crate::config::get_ollama_host();
    let api_key = crate::config::get_api_key(provider.key_name());
    AiSettings {
        provider,
        model,
        ollama_host: host,
        api_key,
    }
}

pub fn api_key_set(provider: Provider) -> bool {
    !crate::config::get_api_key(provider.key_name()).is_empty()
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

fn strip_fences(raw: &str) -> String {
    let mut clean = raw.trim().to_string();
    if clean.starts_with("```") {
        let parts: Vec<&str> = clean.split("```").collect();
        if parts.len() > 1 {
            clean = parts[1].to_string();
        }
        if clean.starts_with("json") {
            clean = clean[4..].to_string();
        }
    }
    clean.trim().trim_end_matches('`').trim().to_string()
}

fn clean_json(raw: &str) -> Result<Value, String> {
    let clean = strip_fences(raw);
    serde_json::from_str(&clean).map_err(|e| {
        let preview: String = raw.chars().take(200).collect();
        format!("Unparseable JSON: {e}\nRaw: {preview}")
    })
}

/// Unified text chat against the active provider.
pub async fn chat(
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
) -> Result<String, String> {
    let s = settings();
    match s.provider {
        Provider::Ollama => crate::ollama::chat(prompt, system, max_tokens, temperature).await,
        Provider::OpenAI | Provider::DeepSeek | Provider::OpenRouter | Provider::AirLlm => {
            openai_compatible_chat(&s, prompt, system, max_tokens, temperature, false, &mut |_| {}).await
        }
        Provider::Gemini => gemini_chat(&s, prompt, system, max_tokens, temperature, false, &mut |_| {}).await,
        Provider::Anthropic => anthropic_chat(&s, prompt, system, max_tokens, temperature, false, &mut |_| {}).await,
    }
}

/// Unified JSON-mode chat against the active provider.
pub async fn chat_json(
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
) -> Result<Value, String> {
    let system = system.unwrap_or("Return ONLY valid JSON. No markdown fences, no extra text, no explanation.");
    let raw = chat(prompt, Some(system), max_tokens, 0.2).await?;
    clean_json(&raw)
}

/// Streamed text chat: token deltas are forwarded to `on_text` as they arrive.
/// Returns the full accumulated reply.
pub async fn chat_stream(
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    mut on_text: impl FnMut(&str) + Send,
) -> Result<String, String> {
    let s = settings();
    match s.provider {
        Provider::Ollama => crate::ollama::chat_stream(prompt, system, max_tokens, temperature, &mut on_text).await,
        Provider::OpenAI | Provider::DeepSeek | Provider::OpenRouter | Provider::AirLlm => {
            openai_compatible_chat(&s, prompt, system, max_tokens, temperature, true, &mut on_text).await
        }
        Provider::Gemini => {
            gemini_chat(&s, prompt, system, max_tokens, temperature, true, &mut on_text).await
        }
        Provider::Anthropic => {
            anthropic_chat(&s, prompt, system, max_tokens, temperature, true, &mut on_text).await
        }
    }
}

/// Unified vision (image + prompt) for providers that support images.
pub async fn vision(
    prompt: &str,
    image_b64: &str,
    mime: &str,
    system: Option<&str>,
    model: Option<&str>,
    max_tokens: u32,
) -> Result<String, String> {
    let s = settings();
    match s.provider {
        Provider::Gemini => {
            let model = model.unwrap_or(&s.model);
            let key = if s.api_key.is_empty() {
                return Err("No Gemini API key configured. Add it in Settings.".to_string());
            } else {
                s.api_key.clone()
            };
            let system_instruction = system.map(|t| json!({"parts": [{"text": t}]}));
            let mut body = json!({
                "contents": [{
                    "role": "user",
                    "parts": [
                        {"text": prompt},
                        {"inline_data": {"mime_type": mime, "data": image_b64}}
                    ]
                }],
                "generationConfig": {"maxOutputTokens": max_tokens, "temperature": 0.4}
            });
            if let Some(si) = system_instruction {
                body["systemInstruction"] = si;
            }
            let client = http_client()?;
            let resp = client
                .post(format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={key}"
                ))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("[Gemini] request error: {e}"))?;
            let status = resp.status();
            let data: Value = resp.json().await.map_err(|e| format!("[Gemini] parse error: {e}"))?;
            if !status.is_success() {
                return Err(format!(
                    "[Gemini] HTTP {status}: {}",
                    data["error"]["message"].as_str().unwrap_or("unknown error")
                ));
            }
            let text = data["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            if text.is_empty() {
                return Err("[Gemini] empty response.".to_string());
            }
            Ok(text)
        }
        Provider::Ollama => {
            let model = model.unwrap_or(&s.model).to_string();
            crate::ollama::chat_with_image(prompt, system, model, image_b64, mime, max_tokens).await
        }
        _ => Err(format!(
            "Vision is only supported with Gemini or a local vision model. Current provider: {}.",
            s.provider.label()
        )),
    }
}

pub async fn vision_from_file(
    prompt: &str,
    image_path: &str,
    system: Option<&str>,
    model: Option<&str>,
    max_tokens: u32,
) -> Result<String, String> {
    let bytes = std::fs::read(image_path)
        .map_err(|e| format!("Could not read image file: {e}"))?;
    let mime = mime_for_path(image_path);
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    vision(prompt, &b64, &mime, system, model, max_tokens).await
}

fn mime_for_path(path: &str) -> String {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "webp" => "image/webp".to_string(),
        "gif" => "image/gif".to_string(),
        _ => "image/png".to_string(),
    }
}

/// Lists models available on the configured Ollama host.
pub async fn list_ollama_models() -> Vec<String> {
    let host = crate::config::get_ollama_host();
    let client = match http_client() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let resp = client
        .get(format!("{host}/api/tags"))
        .send()
        .await
        .ok();
    let Some(resp) = resp else { return vec![] };
    let data: Value = resp.json().await.ok().unwrap_or(Value::Null);
    data["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Availability probe for the current provider (used by the UI indicator).
pub fn is_online() -> bool {
    let s = settings();
    match s.provider {
        Provider::Ollama => crate::ollama::is_running(),
        Provider::AirLlm => crate::airllm::is_running(),
        Provider::OpenAI
        | Provider::DeepSeek
        | Provider::OpenRouter
        | Provider::Gemini
        | Provider::Anthropic => !s.api_key.is_empty(),
    }
}

/// Status payload shown in config_summary / dashboard.
pub fn status() -> Value {
    let s = settings();
    let airllm_st = crate::airllm::status();
    json!({
        "provider": s.provider.as_str(),
        "engine": s.provider.label(),
        "model": s.model,
        "online": is_online(),
        "loaded": airllm_st["loaded"].as_bool().unwrap_or(false),
        "loading": airllm_st["loading"].as_bool().unwrap_or(false),
        "ready": airllm_st["ready"].as_bool().unwrap_or(false),
        "api_key_set": api_key_set(s.provider),
    })
}

/// Attempts a tiny round-trip against the active provider, returning a human
/// friendly result plus measured latency.
pub async fn test_connection() -> Result<Value, String> {
    let s = settings();
    let started = std::time::Instant::now();
    let reply = match s.provider {
        Provider::Ollama => {
            if !crate::ollama::is_running() {
                return Err(format!(
                    "Ollama is not reachable at {}. Start Ollama (or fix the host in Settings) and try again.",
                    s.ollama_host
                ));
            }
            crate::ollama::chat("Reply with the single word: pong", None, 16, 0.0).await?
        }
        Provider::AirLlm => {
            if !crate::airllm::is_running() {
                return Err(
                    "The AirLLM server is not running. Start it from Settings (AI ENGINE section) and try again."
                        .to_string(),
                );
            }
            chat("Reply with the single word: pong", None, 32, 0.0).await?
        }
        Provider::OpenAI
        | Provider::DeepSeek
        | Provider::OpenRouter
        | Provider::Gemini
        | Provider::Anthropic => {
            if s.api_key.is_empty() {
                return Err(format!(
                    "No API key stored for {}. Paste your {} key in Settings and Save.",
                    s.provider.label(),
                    s.provider.label()
                ));
            }
            chat("Reply with the single word: pong", None, 16, 0.0).await?
        }
    };
    let ms = started.elapsed().as_millis();
    Ok(json!({
        "ok": true,
        "provider": s.provider.label(),
        "model": s.model,
        "reply": reply,
        "latency_ms": ms,
    }))
}

pub fn available_models() -> Value {
    let s = settings();
    json!({
        "engine": s.provider.as_str(),
        "provider": s.provider.label(),
        "text_models": [s.model],
        "total_text": 1,
        "total_vision": 0,
    })
}

// ── OpenAI-compatible (OpenAI / DeepSeek / OpenRouter) ──────────────────

fn openai_compatible_endpoint(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAI => "https://api.openai.com/v1/chat/completions",
        Provider::DeepSeek => "https://api.deepseek.com/v1/chat/completions",
        Provider::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
        Provider::AirLlm => crate::airllm::chat_endpoint(),
        _ => unreachable!(),
    }
}

async fn openai_compatible_chat(
    s: &AiSettings,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    stream: bool,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if s.provider.needs_api_key() && s.api_key.is_empty() {
        return Err(format!(
            "No API key configured for {}. Add it in Settings.",
            s.provider.label()
        ));
    }
    let messages = match system {
        Some(sys) => json!([
            {"role": "system", "content": sys},
            {"role": "user", "content": prompt}
        ]),
        None => json!([{"role": "user", "content": prompt}]),
    };
    let body = json!({
        "model": s.model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": stream,
    });
    let timeout = if s.provider == Provider::AirLlm {
        std::time::Duration::from_secs(1800)
    } else {
        REQUEST_TIMEOUT
    };
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.post(openai_compatible_endpoint(s.provider)).json(&body);
    if !s.api_key.is_empty() {
        req = req.bearer_auth(&s.api_key);
    }
    let resp = req.send().await.map_err(|e| format!("[{}] request error: {e}", s.provider.label()))?;
    let status = resp.status();
    if !status.is_success() {
        let data: Value = resp.json().await.unwrap_or(Value::Null);
        let msg = data["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("[{}] HTTP {status}: {msg}", s.provider.label()));
    }
    if !stream {
        let data: Value = resp
            .json()
            .await
            .map_err(|e| format!("[{}] parse error: {e}", s.provider.label()))?;
        let text = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(format!("[{}] empty response.", s.provider.label()));
        }
        return Ok(text);
    }
    // SSE stream
    let mut full = String::new();
    let mut buffer = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("[{}] stream error: {e}", s.provider.label()))?;
        buffer.extend_from_slice(&chunk);
        while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    return Ok(full);
                }
                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                        if !t.is_empty() {
                            full.push_str(t);
                            on_text(t);
                        }
                    }
                }
            }
        }
    }
    if full.is_empty() {
        Err(format!("[{}] empty streamed response.", s.provider.label()))
    } else {
        Ok(full)
    }
}

// ── Gemini ──────────────────────────────────────────────────────────────

async fn gemini_chat(
    s: &AiSettings,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    stream: bool,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if s.api_key.is_empty() {
        return Err("No Gemini API key configured. Add it in Settings.".to_string());
    }
    let mut body = json!({
        "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        "generationConfig": {"maxOutputTokens": max_tokens, "temperature": temperature}
    });
    if let Some(sys) = system {
        body["systemInstruction"] = json!({"parts": [{"text": sys}]});
    }
    let client = http_client()?;
    let endpoint = if stream {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            s.model, s.api_key
        )
    } else {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            s.model, s.api_key
        )
    };
    let resp = client
        .post(&endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("[Gemini] request error: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let data: Value = resp.json().await.unwrap_or(Value::Null);
        let msg = data["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("[Gemini] HTTP {status}: {msg}"));
    }
    let mut full = String::new();
    if !stream {
        let data: Value = resp
            .json()
            .await
            .map_err(|e| format!("[Gemini] parse error: {e}"))?;
        for part in data["candidates"][0]["content"]["parts"].as_array().unwrap_or(&vec![]) {
            if let Some(t) = part["text"].as_str() {
                full.push_str(t);
            }
        }
    } else {
        let mut buffer = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("[Gemini] stream error: {e}"))?;
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        for part in v["candidates"][0]["content"]["parts"]
                            .as_array()
                            .unwrap_or(&vec![])
                        {
                            if let Some(t) = part["text"].as_str() {
                                if !t.is_empty() {
                                    full.push_str(t);
                                    on_text(t);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let text = full.trim().to_string();
    if text.is_empty() {
        return Err("[Gemini] empty response.".to_string());
    }
    Ok(text)
}

// ── Anthropic (Claude) ──────────────────────────────────────────────────

async fn anthropic_chat(
    s: &AiSettings,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f64,
    stream: bool,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    if s.api_key.is_empty() {
        return Err("No Anthropic API key configured. Add it in Settings.".to_string());
    }
    let mut body = json!({
        "model": s.model,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": stream,
        "messages": [{"role": "user", "content": prompt}]
    });
    if let Some(sys) = system {
        body["system"] = json!(sys);
    }
    let client = http_client()?;
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &s.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("[Claude] request error: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let data: Value = resp.json().await.unwrap_or(Value::Null);
        let msg = data["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("[Claude] HTTP {status}: {msg}"));
    }
    let mut full = String::new();
    if !stream {
        let data: Value = resp
            .json()
            .await
            .map_err(|e| format!("[Claude] parse error: {e}"))?;
        for block in data["content"].as_array().unwrap_or(&vec![]) {
            if block["type"].as_str() == Some("text") {
                if let Some(t) = block["text"].as_str() {
                    full.push_str(t);
                }
            }
        }
    } else {
        let mut buffer = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("[Claude] stream error: {e}"))?;
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        match v["type"].as_str() {
                            Some("content_block_delta") => {
                                if let Some(t) = v["delta"]["text"].as_str() {
                                    if !t.is_empty() {
                                        full.push_str(t);
                                        on_text(t);
                                    }
                                }
                            }
                            Some("message_stop") => return Ok(full.trim().to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    let text = full.trim().to_string();
    if text.is_empty() {
        return Err("[Claude] empty response.".to_string());
    }
    Ok(text)
}

/// Kept for provider lookup helpers used elsewhere.
pub static LAST_ENGINE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));