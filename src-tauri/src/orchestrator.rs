use crate::actions;
use crate::agent::task_queue;
use crate::config;
use crate::memory;
use crate::workspace_store::{self, WorkspaceStore};
use regex::Regex;
use serde_json::{json, Value};

/// Maximum tool-call round-trips per user turn before we stop and summarize.
const MAX_TOOL_ITERS: usize = 3;

/// How the model is told to drive tools. Kept terse for small local models.
const TOOL_INSTRUCTIONS: &str = "\
You can take real actions on this Windows PC by calling tools.\n\
Reply with EXACTLY ONE JSON object and NOTHING else — no prose, no markdown fences.\n\
To call a tool: {\"tool\": \"<name>\", \"parameters\": { ... }}\n\
When the request is handled, or no tool is needed, reply: {\"final\": \"<short spoken reply>\"}\n\
Only call a tool when the user clearly asks you to DO something on the computer. For greetings, small talk, or questions about you, use {\"final\": ...} and call NO tool.\n\
Examples:\n\
User request: open calculator -> {\"tool\": \"open_app\", \"parameters\": {\"name\": \"calculator\"}}\n\
User request: how are you -> {\"final\": \"Doing great, sir. How can I help?\"}\n\
Call one tool at a time; you will see its result before deciding the next step.";

/// Only tools that actually do something real today are advertised, so a small
/// model does not waste turns calling stubs. Extend as more actions are ported.
const TOOL_CATALOG: &str = "\
- open_app — open/launch an application. parameters: {\"name\": \"<app>\"}\n\
- web_search — search the web for information. parameters: {\"query\": \"<text>\"}\n\
- weather_report — current weather. parameters: {\"location\": \"<city>\"}\n\
- youtube_video — play or search YouTube. parameters: {\"action\": \"play|search\", \"query\": \"<text>\"}\n\
- send_message — send a chat message. parameters: {\"platform\": \"whatsapp|telegram\", \"receiver\": \"<name>\", \"message\": \"<text>\"}\n\
- computer_control — mouse/keyboard/clipboard/screenshot. parameters: {\"action\": \"type|press|hotkey|click|scroll|screenshot|copy|paste|get_clipboard|focus_window\", \"text\": \"<optional>\"}\n\
- computer_settings — system controls. parameters: {\"action\": \"volume_up|volume_down|mute|volume_set|brightness_up|brightness_down|minimize|maximize|show_desktop|close_window|lock|dark_mode|light_mode|open_settings|play_pause|next_track|restart|shutdown\", \"value\": <0-100 for set actions>, \"confirmed\": \"yes for restart/shutdown\"}\n\
- file_controller — file/folder operations. parameters: {\"action\": \"create_file|create_folder|read|write|list|delete|find|disk_usage|info\", \"path\": \"<dir or shortcut: desktop|downloads|documents>\", \"name\": \"<file>\", \"content\": \"<text>\"}\n\
- file_processor — process an uploaded file. parameters: {\"action\": \"summarize|extract_text|info\", \"file_path\": \"<path>\"}\n\
- browser_control — open a page or search in the browser. parameters: {\"action\": \"go_to|search\", \"url\": \"<url>\", \"query\": \"<text>\"}\n\
- reminder — set a reminder. parameters: {\"message\": \"<text>\", \"minutes\": <number>, \"date\": \"<YYYY-MM-DD optional>\", \"time\": \"<HH:MM optional>\"}\n\
- desktop_control — organize desktop or set wallpaper. parameters: {\"action\": \"organize|clean|stats|wallpaper\", \"path\": \"<image path for wallpaper>\", \"confirmed\": \"yes required for organize/clean\"}\n\
- save_memory — remember a durable fact about the user. parameters: {\"category\": \"identity|preferences|notes\", \"key\": \"<k>\", \"value\": \"<v>\"}";

/// Streaming message kinds forwarded to the frontend during a chat turn.
#[derive(Debug, Clone)]
pub enum StreamMsg {
    /// Transient progress line (e.g. "Ran tool: web_search").
    Status(String),
    /// Assistant reply tokens (accumulate in the UI).
    Text(String),
}

pub struct RanveerOrchestrator {
    pub store: &'static WorkspaceStore,
    pub assistant_name: String,
}

impl Default for RanveerOrchestrator {
    fn default() -> Self {
        RanveerOrchestrator::new()
    }
}

impl RanveerOrchestrator {
    pub fn new() -> Self {
        RanveerOrchestrator {
            store: workspace_store::store(),
            assistant_name: "Ranveer".to_string(),
        }
    }

    fn normalize_text(text: &str) -> String {
        let re = Regex::new(r"[^a-z0-9\s%]").unwrap();
        let collapsed = Regex::new(r"\s+").unwrap();
        let lower = text.to_lowercase();
        let cleaned = re.replace_all(&lower, " ").to_string();
        collapsed.replace_all(&cleaned, " ").trim().to_string()
    }

    fn looks_like_smart_home_command(text: &str) -> bool {
        let normalized = Self::normalize_text(text);
        // A smart-home command needs BOTH a device noun and a control verb, so
        // ordinary requests ("switch to dark mode", "open Microsoft Office",
        // "set a reminder to clean my room") are not hijacked.
        let device_words = [
            "fan", "light", "lights", "lamp", "plug", "socket", "bulb", "kasa", "atomberg",
            "room", "bedroom", "living room", "kitchen", "office", "balcony", "bathroom",
            "home device", "smart home", "smart-home", "switch",
        ];
        let verbs = [
            "turn", "switch", "toggle", "set", "dim", "brighten", "adjust", "control",
            "change", "is the", "are the", "turn on", "turn off", "switch on", "switch off",
        ];
        let has_device = device_words.iter().any(|w| normalized.contains(w));
        let has_verb = verbs.iter().any(|v| normalized.contains(v));
        if !has_device || !has_verb {
            return false;
        }
        // Never intercept reminders / memory about rooms or lights.
        if normalized.contains("reminder")
            || normalized.contains("remember")
            || normalized.contains(" note")
        {
            return false;
        }
        true
    }

    fn looks_like_screen_request(text: &str) -> bool {
        let t = Self::normalize_text(text);
        let direct = [
            "what is on my screen",
            "whats on my screen",
            "read my screen",
            "what does my screen say",
            "analyze my screen",
            "analyse my screen",
            "look at my screen",
            "check my screen",
            "what am i looking at",
            "describe my screen",
            "what is on the screen",
            "read the screen",
        ];
        if direct.iter().any(|p| t.contains(p)) {
            return true;
        }
        let screen_words = ["screen", "display", "monitor", "this window", "the page"];
        let request_words = [
            "what", "check", "look", "analyz", "analys", "read", "tell", "see", "describe",
        ];
        screen_words.iter().any(|w| t.contains(w)) && request_words.iter().any(|w| t.contains(w))
    }

    fn looks_like_bc_command(text: &str) -> bool {
        let normalized = Self::normalize_text(text);
        // Only intercept explicit remote-device requests, not bare words like
        // "volume", "battery", "phone" or "device" which belong to the AI engine
        // (e.g. "set volume to 30", "what is my battery percentage").
        let specific = [
            "ranveer connect",
            "pair device",
            "pair my",
            "pairing",
            "connect my phone",
            "connect my tablet",
            "my phone",
            "my mobile",
            "my tablet",
            "my android",
            "my iphone",
            "my laptop",
            "my pc",
            "my computer",
            "device info",
            "remote control",
        ];
        if specific.iter().any(|w| normalized.contains(w)) {
            return true;
        }
        let flashlight = normalized.contains("flashlight")
            && (normalized.contains(" on") || normalized.contains(" off") || normalized.contains("toggle"));
        let battery = normalized.contains("battery")
            && (normalized.contains("phone")
                || normalized.contains("device")
                || normalized.contains("percent")
                || normalized.contains("percentage"));
        let url = normalized.contains("open url") && normalized.contains("phone")
            || normalized.contains("open url") && normalized.contains("tablet")
            || normalized.contains("open url") && normalized.contains("device");
        let launch = normalized.contains("launch app") && normalized.contains("phone")
            || normalized.contains("launch app") && normalized.contains("tablet")
            || normalized.contains("launch app") && normalized.contains("device");
        flashlight || battery || url || launch
    }

    pub fn build_system_prompt(&self) -> String {
        let base = config::load_system_prompt();
        let memory_block = memory::format_memory_for_prompt(Some(&memory::load_memory()));
        let mut prompt = base;
        if let Some(op) = crate::gesture::current_operator() {
            prompt.push_str(&format!(
                "\n\n[OPERATOR]\nThe person you are talking to is {op}. Address them by name when appropriate."
            ));
        }
        if !memory_block.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&memory_block);
        }
        prompt
    }

    /// Voice/chat intent routing for camera, gesture and face features so the
    /// user can control them by simply talking (no manual clicking).
    async fn handle_vision_intents(&self, text: &str) -> Option<String> {
        let low = text.to_lowercase();

        // Face registration: "register my face", "scan my face and save", "save my face as X"
        let wants_register = [
            "register my face",
            "scan my face",
            "save my face",
            "add my face",
            "remember my face",
            "learn my face",
            "store my face",
            "teach my face",
        ];
        let wants_identify = [
            "who am i",
            "identify me",
            "recognize me",
            "do you know me",
            "know who i am",
            "what is my name",
            "identify my face",
        ];

        let normalized = Self::normalize_text(text);
        let norm_low = normalized.as_str();

        if norm_low.contains("gesture") && (norm_low.contains("stop") || norm_low.contains("off") || norm_low.contains("disable")) {
            crate::gesture::gesture_stop();
            return Some("Gesture control stopped, sir.".to_string());
        }
        if norm_low.contains("gesture") && (norm_low.contains("start") || norm_low.contains("on") || norm_low.contains("enable") || norm_low.contains("activate")) {
            match crate::gesture::gesture_start() {
                Ok(msg) => return Some(msg),
                Err(e) => return Some(format!("Could not start gesture control: {e}")),
            }
        }

        if wants_register.iter().any(|w| low.contains(w)) {
            let name = Self::extract_face_name(text);
            return Some(match crate::gesture::register_face_async(&name).await {
                Ok(res) if res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) => {
                    format!("Done, sir. I've registered your face as {name}. I'll recognize you from now on.")
                }
                Ok(res) => format!(
                    "I couldn't capture enough face samples: {}",
                    res.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error")
                ),
                Err(e) => format!("Face registration failed: {e}"),
            });
        }

        if wants_identify.iter().any(|w| low.contains(w)) {
            return Some(match crate::gesture::identify_face_async().await {
                Ok(res) => {
                    let name = res.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if name.is_empty() {
                        "I don't recognize you yet, sir. Say 'register my face' so I can learn who you are.".to_string()
                    } else {
                        format!("Hello, {name}! Nice to see you again.")
                    }
                }
                Err(e) => format!("Face identification failed: {e}"),
            });
        }

        None
    }

    /// Extracts a name from phrases like "register my face as Veer", "save my
    /// face, my name is Veer", or "scan my face". Falls back to the last known
    /// operator, else "Operator".
    fn extract_face_name(text: &str) -> String {
        let markers = [" as ", " name is ", " name's ", " call me ", " i am ", " i'm ", " for "];
        for marker in markers {
            if let Some(idx) = text.to_lowercase().find(marker) {
                let rest = text[idx + marker.len()..].trim();
                if let Some(first) = rest.split_whitespace().next() {
                    let cleaned: String = first
                        .chars()
                        .filter(|c| c.is_alphabetic())
                        .collect();
                    if cleaned.len() >= 2 {
                        return cleaned;
                    }
                }
            }
        }
        crate::gesture::current_operator().unwrap_or_else(|| "Operator".to_string())
    }

    pub async fn reply(&mut self, text: &str) -> String {
        self.reply_inner(text, None).await
    }

    /// Same as `reply` but streams progress + reply tokens to `emit` as they are
    /// produced (used by the streaming chat command).
    pub async fn reply_stream(&mut self, text: &str, emit: &mut (dyn FnMut(StreamMsg) + Send)) -> String {
        self.reply_inner(text, Some(emit)).await
    }

    async fn reply_inner(
        &mut self,
        text: &str,
        mut emit: Option<&mut (dyn FnMut(StreamMsg) + Send)>,
    ) -> String {
        println!("[Orch] reply start");
        let memory_ctx = self.store.memory_context(text, 5);
        println!("[Orch] memory_ctx len={}", memory_ctx.len());
        let routed = if memory_ctx.is_empty() {
            text.to_string()
        } else {
            format!("{memory_ctx}\n\nCurrent User Request:\n{text}")
        };

        if Self::looks_like_bc_command(text) {
            println!("[Orch] -> bc_command");
            let reply =
                "Ranveer Connect remote device control is available from the Connect page. Use 'connect pair device' for phone pairing.".to_string();
            self.record_assistant(&reply, text);
            return reply;
        }
        if Self::looks_like_smart_home_command(text) {
            println!("[Orch] -> smart_home");
            let reply = "Smart-home control needs a connected provider account (Kasa, Atomberg, Hue, etc.). Add one from the Smart Home page first.".to_string();
            self.record_assistant(&reply, text);
            return reply;
        }

        // AI-driven vision/gesture/face intents (no manual clicks needed).
        if let Some(reply) = self.handle_vision_intents(text).await {
            if let Some(e) = emit.as_mut() {
                e(StreamMsg::Text(reply.clone()));
            }
            self.record_assistant(&reply, text);
            return reply;
        }

        // Screen reading is routed by keyword (not model tool-calling) so it works
        // reliably even on small local models.
        if Self::looks_like_screen_request(text) {
            let reply = crate::vision::answer_about_screen(text).await;
            if let Some(e) = emit.as_mut() {
                e(StreamMsg::Text(reply.clone()));
            }
            self.record_assistant(&reply, text);
            return reply;
        }

        // Tool-calling loop: the model may call real actions and then reply
        // naturally. Offline / plain-chat fallbacks are handled inside.
        let start = std::time::Instant::now();
        let reply = self.run_tool_loop(&routed, emit).await;
        println!("[Orch] reply len={} in {:?}", reply.len(), start.elapsed());
        self.record_assistant(&reply, text);

        // Background: learn durable facts about the user from this turn. Skipped
        // for AirLLM: CPU generation is slow and serialized by the server's lock,
        // so a background extraction would stall the next user chat.
        if config::get_ai_provider() != "airllm" {
            let user_owned = text.to_string();
            let reply_owned = reply.clone();
            tokio::spawn(async move {
                memory::maybe_extract(&user_owned, &reply_owned).await;
            });
        }

        reply
    }

    /// Turns a provider error into a short, friendly explanation for the user.
    fn friendly_ai_error(e: &str) -> String {
        let low = e.to_lowercase();
        if low.contains("api key") {
            "no API key is configured for the selected provider — add one in Settings.".to_string()
        } else if low.contains("401") || low.contains("unauthorized") {
            "the provider rejected the API key (401). Check it in Settings.".to_string()
        } else if low.contains("429") || low.contains("rate limit") {
            "the provider rate-limited the request (429). Try again in a moment.".to_string()
        } else if low.contains("404") || low.contains("model") {
            format!("the model may not exist or the provider changed it ({e})")
        } else if low.contains("connect") || low.contains("timeout") || low.contains("unreachable") {
            "the AI service is unreachable right now. Check your internet or the Ollama host in Settings.".to_string()
        } else {
            format!("the AI engine reported an error: {e}")
        }
    }

    /// ReAct-style tool loop, the primary reasoning path for small local models
    /// (native `/api/chat` tool-calling is layered on later for capable models).
    /// The model emits one JSON object per step — either a tool call or a final
    /// answer — and sees each tool's result before the next step.
    async fn run_tool_loop(
        &self,
        user_text: &str,
        mut emit: Option<&mut (dyn FnMut(StreamMsg) + Send)>,
    ) -> String {
        let base_system = self.build_system_prompt();
        let system = format!("{base_system}\n\n{TOOL_INSTRUCTIONS}\n\nAvailable tools:\n{TOOL_CATALOG}");

        let mut history = format!("User request: {user_text}\n\nRespond with one JSON object.");
        let mut observations: Vec<String> = Vec::new();

        for iter in 0..MAX_TOOL_ITERS {
            if let Some(e) = emit.as_mut() {
                e(StreamMsg::Status(if iter == 0 {
                    "Thinking…".to_string()
                } else {
                    format!("Verifying step {iter}…")
                }));
            }
            let raw = match crate::ai::chat(&history, Some(&system), 220, 0.3).await {
                Ok(r) => r,
                Err(e) => {
                    if iter == 0 && observations.is_empty() {
                        println!("[ToolLoop] AI unreachable: {e}");
                        let msg =
                            format!("I couldn't reach the AI engine, sir. {}", Self::friendly_ai_error(&e));
                        if let Some(em) = emit.as_mut() {
                            em(StreamMsg::Text(msg.clone()));
                        }
                        return msg;
                    }
                    break;
                }
            };

            let action = match parse_action(&raw) {
                Some(a) => a,
                None => {
                    // Model replied in prose. If nothing was done yet, that IS the
                    // answer; otherwise fall through to a natural summary.
                    if observations.is_empty() {
                        let reply = raw.trim().to_string();
                        if let Some(e) = emit.as_mut() {
                            e(StreamMsg::Text(reply.clone()));
                        }
                        return reply;
                    }
                    break;
                }
            };

            // A final answer in any of the shapes a small model tends to emit
            // ({"final":...}, {"response":...}, {"answer":...}, ...).
            if let Some(final_msg) = extract_final(&action) {
                if observations.is_empty() {
                    if let Some(e) = emit.as_mut() {
                        e(StreamMsg::Text(final_msg.clone()));
                    }
                    return final_msg;
                }
                break;
            }

            let tool = action
                .get("tool")
                .or_else(|| action.get("tool_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if tool.is_empty() {
                if observations.is_empty() {
                    // Valid JSON but no recognizable tool/answer — return its text
                    // if any, else the raw text.
                    let reply = raw.trim().to_string();
                    if let Some(e) = emit.as_mut() {
                        e(StreamMsg::Text(reply.clone()));
                    }
                    return reply;
                }
                break;
            }

            let params = action
                .get("parameters")
                .or_else(|| action.get("args"))
                .or_else(|| action.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            println!("[ToolLoop] iter {iter}: tool '{tool}' args {params}");
            if let Some(e) = emit.as_mut() {
                e(StreamMsg::Status(format!("Running tool: {tool}…")));
            }
            let result = self.execute_tool(&tool, &params).await;
            observations.push(format!("{tool} -> {result}"));

            history = format!(
                "User request: {user_text}\n\nActions taken so far:\n{}\n\nIf the request is fully handled, reply {{\"final\": \"<short spoken reply>\"}}. Otherwise call the next tool as one JSON object.",
                observations
                    .iter()
                    .map(|o| format!("- {o}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }

        // No tool ran and no final produced → plain conversational reply.
        if observations.is_empty() {
            return self
                .stream_llm_reply(user_text, 512, 0.6, emit)
                .await;
        }

        // Actions ran but the model never gave a clean final → summarize them.
        let summary_prompt = format!(
            "The user asked: \"{user_text}\".\nYou performed these actions with results:\n{}\n\nReply to the user in one or two natural sentences describing what you did. Do NOT output JSON.",
            observations
                .iter()
                .map(|o| format!("- {o}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        self.stream_llm_reply(&summary_prompt, 300, 0.5, emit).await
    }

    /// Runs a final natural-language reply, streaming tokens when an emitter is
    /// attached (cloud providers) and falling back to a one-shot call otherwise.
    async fn stream_llm_reply(
        &self,
        prompt: &str,
        max_tokens: u32,
        temperature: f64,
        emit: Option<&mut (dyn FnMut(StreamMsg) + Send)>,
    ) -> String {
        let system = self.build_system_prompt();
        match emit {
            Some(e) => {
                match crate::ai::chat_stream(prompt, Some(&system), max_tokens, temperature, |t| {
                    e(StreamMsg::Text(t.to_string()))
                })
                .await
                {
                    Ok(r) => r.trim().to_string(),
                    Err(err) => {
                        println!("[ToolLoop] stream failed, one-shot fallback: {err}");
                        crate::ai::chat(prompt, Some(&system), max_tokens, temperature)
                            .await
                            .map(|r| r.trim().to_string())
                            .unwrap_or_else(|e| {
                                format!(
                                    "I couldn't complete that, sir. {}",
                                    Self::friendly_ai_error(&e)
                                )
                            })
                    }
                }
            }
            None => match crate::ai::chat(prompt, Some(&system), max_tokens, temperature).await {
                Ok(r) => r.trim().to_string(),
                Err(e) => format!("I couldn't complete that, sir. {}", Self::friendly_ai_error(&e)),
            },
        }
    }

    /// Executes a tool by name. Kept local-only: dispatched tools that need
    /// cloud vision return a friendly "unavailable" message.

    /// Dispatches a named tool with its parameters (mirrors Python _execute_tool,
    /// minus the audio/UI side-effects).
    pub async fn execute_tool(&self, name: &str, args: &Value) -> String {
        match name {
            "save_memory" => {
                let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("notes");
                let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
                let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
                if key.is_empty() || value.is_empty() {
                    return "Couldn't save memory: both key and value are required.".to_string();
                }
                memory::update_memory(&json!({ category: { key: { "value": value } } }));
                println!("[Memory] save_memory: {category}/{key} = {value}");
                "Memory saved.".to_string()
            }
            "agent_task" => {
                let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let task_id = task_queue::get_queue().submit(&goal);
                format!("Task started (ID: {task_id}).")
            }
            "shutdown_ranveer" => {
                "I can't shut myself down from inside a tool call — ask your operating system instead.".to_string()
            }
            "claude_code" => {
                let desc = args
                    .get("description")
                    .or_else(|| args.get("task"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if desc.is_empty() {
                    return "claude_code needs a description of the code to generate.".to_string();
                }
                let system = "You are a code generator for a personal desktop assistant. Write complete, working, well-structured code for the requested task. Respond with ONLY the code, no explanation, no markdown fences.";
                match crate::or_client::client().chat(&desc, Some(system), None, 2048, 0.3).await {
                    Ok(code) => {
                        let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
                        let base = args
                            .get("workspace_path")
                            .and_then(|v| v.as_str())
                            .filter(|p| !p.is_empty())
                            .map(std::path::PathBuf::from)
                            .or_else(|| dirs::desktop_dir().map(|d| d.join("ranveer_generated")))
                            .unwrap_or_else(|| std::path::PathBuf::from("ranveer_generated"));
                        let dir = base.join(&stamp);
                        let saved = std::fs::create_dir_all(&dir)
                            .and_then(|_| std::fs::write(dir.join("generated.txt"), &code))
                            .is_ok();
                        if saved {
                            format!(
                                "Generated the code and saved it to {} (generated.txt).",
                                dir.display()
                            )
                        } else {
                            format!(
                                "Generated the code ({code} chars) but could not write it to {}.",
                                dir.display()
                            )
                        }
                    }
                    Err(e) => format!("Code generation failed: {e}"),
                }
            }
            "smart_home_control" => {
                let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                if command.is_empty() {
                    return "No smart-home command provided.".to_string();
                }
                "Smart-home providers need to be configured (see Smart Home page).".to_string()
            }
            "screen_process" => {
                let question = args
                    .get("question")
                    .or_else(|| args.get("query"))
                    .or_else(|| args.get("text"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("What is on my screen?");
                crate::vision::answer_about_screen(question).await
            }
            "browser_control" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("go_to");
                let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
                match action {
                    "go_to" if !url.is_empty() => {
                        open_browser(url);
                        format!("Opened {url} in the browser.")
                    }
                    "search" => {
                        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                        let url = format!(
                            "https://www.google.com/search?q={}",
                            query.replace(' ', "+")
                        );
                        open_browser(&url);
                        format!("Searching for {query}.")
                    }
                    _ => format!(
                        "browser_control '{action}' isn't wired up yet. Supported actions: go_to, search."
                    ),
                }
            }
            "computer_settings" => actions::computer_settings::computer_settings(args.clone()),
            "send_message" => actions::more::send_message(args.clone()),
            "youtube_video" => actions::more::youtube_video(args.clone()),
            "flight_finder" => actions::more::flight_finder(args.clone()),
            "game_updater" => actions::more::game_updater(args.clone()),
            "computer_control" => actions::more::computer_control(args.clone()),
            "file_processor" => {
                let mut a = args.clone();
                if a.get("file_path").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                    if let Value::String(s) = a["last_file"].clone() {
                        a["file_path"] = json!(s);
                    }
                }
                actions::file_processor::file_processor(a)
            }
            "file_controller" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                match action {
                    "organize_desktop" | "organize_folder" => {
                        "Folder organization isn't available yet in this build — use desktop_control organize instead (it asks for your confirmation).".to_string()
                    }
                    _ => actions::agent_executor_file_controller(args).unwrap_or_else(|e| e),
                }
            }
            // Registry actions are synchronous and some (e.g. web_search) use a
            // blocking HTTP client, so run them off the async runtime.
            _ => {
                let n = name.to_string();
                let a = args.clone();
                match tokio::task::spawn_blocking(move || actions::execute(&n, a)).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(e)) => {
                        println!("[Ranveer] Unknown or unported tool '{name}': {e}");
                        format!("Unknown action: {name}")
                    }
                    Err(_) => "That action failed to run.".to_string(),
                }
            }
        }
    }

    fn record_assistant(&self, content: &str, user_text: &str) {
        println!("[Record] recording user turn...");
        self.store.record_chat("user", user_text, None, None);
        println!("[Record] recording assistant turn...");
        self.store.record_chat("assistant", &content, None, None);
        println!("[Record] done");
    }
}

/// Pulls a final spoken answer out of whatever JSON shape a small model emits
/// ({"final":...}, {"response":...}, {"answer":...}, ...). None if it looks like
/// a tool call instead.
fn extract_final(action: &Value) -> Option<String> {
    if action.get("tool").is_some() || action.get("tool_name").is_some() {
        return None;
    }
    for key in [
        "final", "response", "answer", "reply", "content", "message", "text", "result", "output",
    ] {
        if let Some(s) = action.get(key).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Extracts the first balanced JSON object from a model reply, tolerating code
/// fences and surrounding prose that small local models often add.
fn parse_action(raw: &str) -> Option<Value> {
    let mut s = raw.trim();

    // Strip a leading ```json / ``` fence if present.
    if let Some(idx) = s.find("```") {
        let after = &s[idx + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```") {
            s = after[..end].trim();
        } else {
            s = after.trim();
        }
    }

    // Scan for the first balanced { ... } object.
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut end = None;
    for i in start..s.len() {
        let c = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    serde_json::from_str::<Value>(&s[start..=end]).ok()
}

pub fn open_browser(url: &str) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().is_ok()
    }
}