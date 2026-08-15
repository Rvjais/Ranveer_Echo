//! LLM client used by the agent planner/executor.
//!
//! Delegates every call to the active provider configured in Settings
//! (Ollama / OpenAI / DeepSeek / OpenRouter / Gemini / Claude).
//! The public API is kept stable so agent code never needs to change.

use once_cell::sync::Lazy;
use serde_json::{json, Value};

pub struct LlmClient;

impl LlmClient {
    pub fn new() -> Self {
        LlmClient
    }

    pub async fn chat(
        &self,
        prompt: &str,
        system: Option<&str>,
        _model: Option<&str>,
        max_tokens: u32,
        temperature: f64,
    ) -> Result<String, String> {
        crate::ai::chat(prompt, system, max_tokens, temperature).await
    }

    pub async fn chat_json(
        &self,
        prompt: &str,
        system: Option<&str>,
        _model: Option<&str>,
        max_tokens: u32,
    ) -> Result<Value, String> {
        crate::ai::chat_json(prompt, system, max_tokens).await
    }

    pub async fn vision(
        &self,
        prompt: &str,
        image_b64: &str,
        mime: &str,
        system: Option<&str>,
        model: Option<&str>,
        max_tokens: u32,
    ) -> Result<String, String> {
        crate::ai::vision(prompt, image_b64, mime, system, model, max_tokens).await
    }

    pub async fn vision_from_file(
        &self,
        prompt: &str,
        image_path: &str,
        system: Option<&str>,
        model: Option<&str>,
        max_tokens: u32,
    ) -> Result<String, String> {
        crate::ai::vision_from_file(prompt, image_path, system, model, max_tokens).await
    }

    pub async fn multi_turn(
        &self,
        messages: &[Value],
        _model: Option<&str>,
        max_tokens: u32,
        temperature: f64,
    ) -> Result<String, String> {
        let prompt = messages
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n");
        let system = messages
            .iter()
            .find(|m| m["role"].as_str() == Some("system"))
            .and_then(|m| m.get("content").and_then(|c| c.as_str()));
        crate::ai::chat(&prompt, system, max_tokens, temperature).await
    }

    pub fn available_models(&self) -> Value {
        crate::ai::available_models()
    }
}

pub fn client() -> &'static LlmClient {
    static CLIENT: Lazy<LlmClient> = Lazy::new(LlmClient::new);
    &CLIENT
}

/// Legacy alias kept for callers that referenced the old type name.
pub type OpenRouterClient = LlmClient;

#[allow(dead_code)]
fn _keep_json_import() -> Value {
    json!({})
}