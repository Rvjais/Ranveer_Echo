#![allow(dead_code)]

mod actions;
mod agent;
mod ai;
mod airllm;
mod ranveer_connect;
mod config;
mod dashboard;
mod gesture;
mod llm;
mod memory;
mod ollama;
mod or_client;
mod orchestrator;
mod smart_home;
mod vision;
mod voice;
mod workspace_store;

use serde_json::Value;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager, State};

struct AppState {
    orchestrator: tokio::sync::Mutex<orchestrator::RanveerOrchestrator>,
}

impl AppState {
    fn new() -> Self {
        AppState {
            orchestrator: tokio::sync::Mutex::new(orchestrator::RanveerOrchestrator::new()),
        }
    }
}

#[tauri::command]
async fn chat(text: String, state: State<'_, AppState>) -> Result<String, String> {
    println!("[Chat] >>> received: {text}");
    let mut orch = state.orchestrator.lock().await;
    println!("[Chat] lock acquired, calling reply...");
    let timeout_secs = if config::get_ai_provider() == "airllm" { 1800 } else { 180 };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        orch.reply(&text),
    )
    .await;
    match result {
        Ok(reply) => {
            println!("[Chat] <<< reply len={}", reply.len());
            Ok(reply)
        }
        Err(_) => {
            println!("[Chat] !!! TIMED OUT");
            Err("Request timed out. The model may be slow or unreachable.".to_string())
        }
    }
}

/// Streaming chat: emits `chat-stream` events (`{kind: "status"|"text", data}`)
/// as the turn progresses, then returns the complete reply.
#[tauri::command]
async fn chat_stream(text: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    println!("[ChatStream] >>> received: {text}");
    let mut orch = state.orchestrator.lock().await;
    let timeout_secs = if config::get_ai_provider() == "airllm" { 1800 } else { 180 };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        orch.reply_stream(&text, &mut |msg| {
            let (kind, data) = match msg {
                orchestrator::StreamMsg::Status(s) => ("status", s),
                orchestrator::StreamMsg::Text(s) => ("text", s),
            };
            let _ = app.emit("chat-stream", serde_json::json!({"kind": kind, "data": data}));
        }),
    )
    .await;
    match result {
        Ok(reply) => {
            println!("[ChatStream] <<< reply len={}", reply.len());
            Ok(reply)
        }
        Err(_) => {
            println!("[ChatStream] !!! TIMED OUT");
            Err("Request timed out. The model may be slow or unreachable.".to_string())
        }
    }
}

/// Tests the active provider (Ollama host reachable, or API key accepted) and
/// returns the resolved engine name, model, and online status.
#[tauri::command]
async fn ai_test_connection() -> Result<Value, String> {
    crate::ai::test_connection().await
}

/// Lists models installed on the configured Ollama host.
#[tauri::command]
async fn ai_list_ollama_models() -> Value {
    serde_json::to_value(crate::ai::list_ollama_models().await).unwrap_or(Value::Null)
}

#[tauri::command]
fn config_set_ai_provider(provider: String) -> Result<Value, String> {
    config::set_ai_provider(&provider)?;
    Ok(serde_json::json!({
        "ai_provider": config::get_ai_provider(),
        "ai_model": config::get_ai_model(&crate::ai::Provider::from_str(&config::get_ai_provider())),
    }))
}

#[tauri::command]
fn config_set_ai_model(model: String) -> Result<Value, String> {
    config::set_ai_model(&model)?;
    crate::ollama::invalidate_model_cache();
    Ok(serde_json::json!({ "ai_model": model }))
}

#[tauri::command]
fn config_set_airllm_model(model: String) -> Result<Value, String> {
    config::set_airllm_model(&model)?;
    crate::ollama::invalidate_model_cache();
    Ok(serde_json::json!({ "airllm_model": model }))
}

#[tauri::command]
fn config_set_ollama_host(host: String) -> Result<Value, String> {
    config::set_ollama_host(&host)?;
    crate::ollama::invalidate_model_cache();
    Ok(serde_json::json!({ "ollama_host": config::get_ollama_host() }))
}

#[tauri::command]
fn airllm_start(model: Option<String>) -> Result<Value, String> {
    crate::airllm::start(model)
}

#[tauri::command]
fn airllm_stop() -> Result<Value, String> {
    crate::airllm::stop()
}

#[tauri::command]
fn airllm_status() -> Value {
    crate::airllm::status()
}

#[tauri::command]
async fn airllm_install(model: String) -> Result<Value, String> {
    crate::airllm::install(&model).await
}

#[tauri::command]
fn store_new_conversation(title: String) -> String {
    workspace_store::store().create_conversation(&title)
}

#[tauri::command]
fn store_activate_conversation(conversation_id: String) {
    workspace_store::store().set_active_conversation_id(&conversation_id);
}

#[tauri::command]
fn run_action(name: String, parameters: Value) -> Result<String, String> {
    actions::execute(&name, parameters)
}

#[tauri::command]
fn memory_all() -> Value {
    memory::load_memory()
}

#[tauri::command]
fn memory_save(category: String, key: String, value: String) -> String {
    memory::remember(&key, &value, &category)
}

#[tauri::command]
fn memory_forget(category: String, key: String) -> String {
    memory::forget(&key, &category)
}

#[tauri::command]
fn memory_prompt_block() -> String {
    memory::format_memory_for_prompt(Some(&memory::load_memory()))
}

#[tauri::command]
fn store_list_conversations(search: String) -> Vec<Value> {
    workspace_store::store().list_conversations(&search)
}

#[tauri::command]
fn store_grouped_conversations(search: String) -> Value {
    workspace_store::store().grouped_conversations(&search)
}

#[tauri::command]
fn store_get_conversation(conversation_id: String) -> Option<Value> {
    workspace_store::store().get_conversation(&conversation_id)
}

#[tauri::command]
fn store_active_conversation() -> Option<String> {
    workspace_store::store().get_active_conversation_id()
}

#[tauri::command]
fn store_rename_conversation(conversation_id: String, title: String) {
    workspace_store::store().rename_conversation(&conversation_id, &title);
}

#[tauri::command]
fn store_pin_conversation(conversation_id: String, pinned: bool) {
    workspace_store::store().pin_conversation(&conversation_id, pinned);
}

#[tauri::command]
fn store_delete_conversation(conversation_id: String) {
    workspace_store::store().delete_conversation(&conversation_id);
}

#[tauri::command]
fn store_record_chat(role: String, content: String) -> String {
    workspace_store::store().record_chat(&role, &content, None, None)
}

#[tauri::command]
fn store_search_memories(query: String, limit: usize) -> Vec<Value> {
    workspace_store::store().search_memories(&query, limit)
}

#[tauri::command]
fn store_all_memories() -> Vec<Value> {
    workspace_store::store().all_memories()
}

#[tauri::command]
fn store_export_conversation(conversation_id: String, path: String) -> Result<String, String> {
    workspace_store::store().export_conversation(&conversation_id, &path)
}

#[tauri::command]
fn smart_home_platforms() -> Value {
    smart_home::service().list_platforms()
}

#[tauri::command]
fn smart_home_devices() -> Value {
    serde_json::to_value(smart_home::service().list_devices()).unwrap_or(Value::Null)
}

#[tauri::command]
fn smart_home_execute_command(command: String) -> Value {
    smart_home::service().execute_command(&command)
}

#[tauri::command]
fn connect_list_devices() -> Value {
    ranveer_connect::get_service().list_devices()
}

#[tauri::command]
fn connect_get_device(query: String) -> Option<Value> {
    ranveer_connect::get_service().get_device(&query)
}

#[tauri::command]
fn connect_gateway_info() -> Value {
    ranveer_connect::get_service().gateway_info()
}

#[tauri::command]
fn connect_pair_offer() -> Value {
    ranveer_connect::get_service().create_pairing_offer()
}

#[tauri::command]
fn connect_pair_device(device: Value) -> Value {
    ranveer_connect::get_service().pair_device(device)
}

#[tauri::command]
fn connect_disconnect_device(device_id: String) -> Value {
    ranveer_connect::get_service().disconnect_device(&device_id)
}

#[tauri::command]
fn connect_revoke_device(device_id: String) -> Value {
    ranveer_connect::get_service().revoke_device(&device_id)
}

#[tauri::command]
fn connect_route_command(target: String, action: String, parameters: Value) -> Value {
    ranveer_connect::get_service().route_command(&target, &action, &parameters)
}

#[tauri::command]
async fn agent_submit_task(goal: String) -> String {
    agent::task_queue::get_queue().submit(&goal)
}

#[tauri::command]
async fn agent_task_status(task_id: String) -> Option<Value> {
    agent::task_queue::get_queue().get_status(&task_id)
}

#[tauri::command]
async fn agent_tasks() -> Vec<Value> {
    agent::task_queue::get_queue().get_all_statuses()
}

#[tauri::command]
async fn agent_cancel_task(task_id: String) -> bool {
    agent::task_queue::get_queue().cancel(&task_id)
}

#[tauri::command]
async fn agent_execute_goal(goal: String) -> String {
    agent::AgentExecutor::new().execute(&goal, None).await.0
}

#[tauri::command]
async fn voice_start(app: tauri::AppHandle, ptt: bool) -> Result<String, String> {
    voice::start(app, ptt).await
}

/// Called by the voice session when a speech utterance is recognized.
pub(crate) async fn handle_voice_text(app: tauri::AppHandle, text: &str) {
    println!("[VoiceChat] user: {text}");
    let _ = app.emit("voice-transcript", serde_json::json!({"role": "user", "text": text}));

    let state = app.state::<AppState>();
    let mut orch = state.orchestrator.lock().await;
    let reply = match tokio::time::timeout(
        std::time::Duration::from_secs(180),
        orch.reply(text),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => "I'm sorry, that took too long. Try again, sir.".to_string(),
    };
    drop(orch);

    println!("[VoiceChat] reply: {reply}");
    let _ = app.emit("voice-transcript", serde_json::json!({"role": "assistant", "text": reply}));

    speak_text(&reply);
}

#[tauri::command]
fn voice_stop() -> String {
    voice::stop()
}

#[tauri::command]
fn voice_state() -> Value {
    serde_json::json!({
        "running": voice::is_running(),
        "speaking": voice::is_speaking(),
        "wake_required": voice::wake_required(),
    })
}

/// Enables/disables wake-word gating for always-listening; returns the new state.
#[tauri::command]
fn voice_set_wake_required(required: bool) -> bool {
    voice::set_wake_required(required);
    voice::wake_required()
}

static TTS_CHILD: once_cell::sync::Lazy<std::sync::Mutex<Option<std::process::Child>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

/// Increments on every speak/stop so a stale TTS waiter thread knows it has been
/// superseded and must not clear the speaking flag out from under a newer one.
static SPEAK_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Speaks text via SAPI. Sets `VOICE.speaking` for the whole utterance (so the
/// UI orb reflects it) and supports barge-in: a newer call kills the previous
/// speech. Fire-and-forget; a small waiter thread clears the speaking flag when
/// the utterance finishes.
pub(crate) fn speak_text(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    // Barge-in: stop any current speech.
    if let Ok(mut guard) = TTS_CHILD.lock() {
        if let Some(mut prev) = guard.take() {
            let _ = prev.kill();
        }
    }
    let tmp = std::env::temp_dir().join(format!("ranveer_say_{}.txt", std::process::id()));
    if std::fs::write(&tmp, text).is_err() {
        return;
    }
    let script = format!(
        "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak((Get-Content -Raw -LiteralPath '{0}'))",
        tmp.to_string_lossy().replace('\'', "''")
    );
    let my_gen = SPEAK_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .spawn();
    if let Ok(child) = child {
        voice::VOICE.speaking.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = TTS_CHILD.lock() {
            *guard = Some(child);
        }
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if SPEAK_GEN.load(Ordering::SeqCst) != my_gen {
                break; // a newer utterance owns the speaking flag now
            }
            let done = if let Ok(mut guard) = TTS_CHILD.lock() {
                match guard.as_mut() {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_)) | Err(_)),
                    None => true,
                }
            } else {
                false
            };
            if done {
                if SPEAK_GEN.load(Ordering::SeqCst) == my_gen {
                    voice::VOICE.speaking.store(false, Ordering::SeqCst);
                    if let Ok(mut g) = TTS_CHILD.lock() {
                        let _ = g.take();
                    }
                }
                break;
            }
        });
    }
}

#[tauri::command]
fn speak(text: String) -> Result<(), String> {
    speak_text(&text);
    Ok(())
}

#[tauri::command]
fn speak_stop() {
    // Supersede any waiter so it won't fight this reset, then kill + clear state.
    SPEAK_GEN.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut guard) = TTS_CHILD.lock() {
        if let Some(mut prev) = guard.take() {
            let _ = prev.kill();
        }
    }
    voice::VOICE.speaking.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn attention_status() -> Value {
    gesture::attention_status()
}

#[tauri::command]
async fn camera_stream_start(app: tauri::AppHandle) -> Result<String, String> {
    gesture::camera_stream_start(app)
}

#[tauri::command]
fn camera_stream_stop() -> String {
    gesture::camera_stream_stop();
    "Camera feed stopped.".to_string()
}

#[tauri::command]
fn gesture_start() -> Result<String, String> {
    gesture::gesture_start()
}

#[tauri::command]
fn gesture_stop() -> String {
    gesture::gesture_stop();
    "Gesture control stopped.".to_string()
}

#[tauri::command]
fn face_list() -> Result<Value, String> {
    gesture::face_list()
}

#[tauri::command]
async fn face_register(name: String) -> Result<Value, String> {
    gesture::register_face_async(&name).await
}

#[tauri::command]
async fn face_identify() -> Result<Value, String> {
    gesture::identify_face_async().await
}

#[tauri::command]
fn config_summary() -> Value {
    let status = crate::ai::status();
    serde_json::json!({
        "os": config::get_os(),
        "developer_mode": config::developer_mode_enabled(),
        "assistant": "Ranveer (Rust)",
        "ai_engine": status["engine"].as_str().unwrap_or("AI"),
        "ai_provider": status["provider"].as_str().unwrap_or(""),
        "ai_model": status["model"].as_str().unwrap_or(""),
        "ai_online": status["online"].as_bool().unwrap_or(false),
        "local_model": crate::ollama::local_model_name(),
        "ollama_running": crate::ollama::is_running(),
        "ollama_host": config::get_ollama_host(),
        "system_prompt_loaded": config::load_system_prompt().len() > 10,
    })
}

#[tauri::command]
fn config_set_api_key(name: String, value: String) -> Result<Value, String> {
    config::set_api_key(&name, &value)?;
    Ok(serde_json::json!(config::get_api_key_safe(&name)))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    dashboard::start();
    if config::get_ai_provider() == "airllm" {
        let _ = crate::airllm::start(None);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            chat,
            chat_stream,
            ai_test_connection,
            ai_list_ollama_models,
            config_set_ai_provider,
            config_set_ai_model,
            config_set_airllm_model,
            config_set_ollama_host,
            airllm_start,
            airllm_stop,
            airllm_status,
            airllm_install,
            store_new_conversation,
            store_activate_conversation,
            run_action,
            memory_all,
            memory_save,
            memory_forget,
            memory_prompt_block,
            store_list_conversations,
            store_grouped_conversations,
            store_get_conversation,
            store_active_conversation,
            store_rename_conversation,
            store_pin_conversation,
            store_delete_conversation,
            store_record_chat,
            store_search_memories,
            store_all_memories,
            store_export_conversation,
            smart_home_platforms,
            smart_home_devices,
            smart_home_execute_command,
            connect_list_devices,
            connect_get_device,
            connect_gateway_info,
            connect_pair_offer,
            connect_pair_device,
            connect_disconnect_device,
            connect_revoke_device,
            connect_route_command,
            agent_submit_task,
            agent_task_status,
            agent_tasks,
            agent_cancel_task,
            agent_execute_goal,
            config_summary,
            config_set_api_key,
            voice_start,
            voice_stop,
            voice_state,
            voice_set_wake_required,
            speak,
            speak_stop,
            attention_status,
            camera_stream_start,
            camera_stream_stop,
            gesture_start,
            gesture_stop,
            face_list,
            face_register,
            face_identify,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}