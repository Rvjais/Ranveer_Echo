//! AirLLM local model server integration.
//!
//! Launches `airllm_server.py` (a small OpenAI-compatible HTTP wrapper around
//! the AirLLM library, repo cloned at `airllm/`) and exposes start / stop /
//! status commands to the frontend. Once running, the AI engine routes
//! `Provider::AirLlm` chat traffic to `http://127.0.0.1:8531/v1/chat/completions`.
//!
//! Console protocol parsed from the server's stdout:
//!   AIRLLM_SERVER_READY port=<p> device=<d> model=<m>
//!   AIRLLM_STATE  <json>
//!   AIRLLM_ERROR  <message>

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub const PORT: u16 = 8531;
const TIMEOUT: Duration = Duration::from_millis(600);

static AIRLLM_CHILD: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));
static READY: AtomicBool = AtomicBool::new(false);
static LAST_STATE: Lazy<Mutex<Value>> = Lazy::new(|| Mutex::new(Value::Null));
static LAST_ERR: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

pub fn base_url() -> String {
    format!("http://127.0.0.1:{PORT}")
}

/// Endpoint the OpenAI-compatible chat path posts to.
pub fn chat_endpoint() -> &'static str {
    "http://127.0.0.1:8531/v1/chat/completions"
}

fn venv_python() -> String {
    let base = crate::config::base_dir();
    let venv = base.join("venv").join("Scripts").join("python.exe");
    if venv.exists() {
        return venv.to_string_lossy().to_string();
    }
    "python".to_string()
}

fn server_script() -> String {
    crate::config::base_dir().join("airllm_server.py").to_string_lossy().to_string()
}

fn tcp_probe() -> bool {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{PORT}").parse().unwrap();
    TcpStream::connect_timeout(&addr, TIMEOUT).is_ok()
}

/// True when the AirLLM server is answering on its port.
pub fn is_running() -> bool {
    tcp_probe()
}

fn owned_child_alive() -> bool {
    let mut guard = AIRLLM_CHILD.lock().unwrap();
    match guard.as_mut() {
        Some(c) => match c.try_wait() {
            Ok(None) => true,
            _ => {
                *guard = None;
                false
            }
        },
        None => false,
    }
}

/// Current server state, combining the health probe with the last state the
/// server reported on stdout.
pub fn status() -> Value {
    let up = tcp_probe();
    let owned = owned_child_alive();
    let state = LAST_STATE.lock().unwrap().clone();
    let err = LAST_ERR.lock().unwrap().clone();
    let ready = READY.load(Ordering::Relaxed) && up;
    json!({
        "running": up,
        "owned": owned,
        "ready": ready,
        "loading": state.get("loading").and_then(|v| v.as_bool()).unwrap_or(false),
        "loaded": state.get("loaded").and_then(|v| v.as_bool()).unwrap_or(false),
        "model": state.get("model").and_then(|v| v.as_str()).unwrap_or(""),
        "device": state.get("device").and_then(|v| v.as_str()).unwrap_or(""),
        "error": err,
        "port": PORT,
    })
}

fn spawn_reader<R: std::io::Read + Send + 'static>(reader: R, is_err: bool) {
    std::thread::spawn(move || {
        let mut buf = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match buf.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let l = line.trim();
                    if l.is_empty() {
                        continue;
                    }
                    if !is_err {
                        if let Some(rest) = l.strip_prefix("AIRLLM_SERVER_READY") {
                            READY.store(true, Ordering::Relaxed);
                            let _ = rest;
                        } else if let Some(rest) = l.strip_prefix("AIRLLM_STATE ") {
                            if let Ok(v) = serde_json::from_str::<Value>(rest) {
                                *LAST_STATE.lock().unwrap() = v;
                            }
                        } else if let Some(rest) = l.strip_prefix("AIRLLM_ERROR ") {
                            *LAST_ERR.lock().unwrap() = rest.to_string();
                        }
                    } else if let Some(rest) = l.strip_prefix("AIRLLM_ERROR ") {
                        *LAST_ERR.lock().unwrap() = rest.to_string();
                    }
                }
            }
        }
    });
}

/// Starts the AirLLM server as a child of this process. The model is loaded
/// lazily on first request, so startup returns as soon as the HTTP port is up.
pub fn start(model: Option<String>) -> Result<Value, String> {
    if tcp_probe() {
        if !owned_child_alive() {
            // An unowned zombie process is running (e.g. from a previous hot-reload).
            // Shut it down via the new /v1/shutdown endpoint so we can bind to the port.
            let client = reqwest::blocking::Client::new();
            let _ = client.post(format!("{}/v1/shutdown", base_url())).send();
            std::thread::sleep(Duration::from_millis(1000));
        } else {
            return Ok(json!({
                "started": false,
                "already_running": true,
                "status": status(),
            }));
        }
    }
    {
        let mut guard = AIRLLM_CHILD.lock().unwrap();
        if let Some(mut c) = guard.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    READY.store(false, Ordering::Relaxed);
    *LAST_ERR.lock().unwrap() = String::new();

    let py = venv_python();
    let script = server_script();
    let shards = crate::config::base_dir().join("config").join("airllm_shards");
    let mut cmd = Command::new(&py);
    cmd.arg(&script)
        .arg("--port")
        .arg(PORT.to_string())
        .arg("--shards-path")
        .arg(shards.to_string_lossy().to_string())
        .arg("--max-seq-len")
        .arg("1024")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .or_else(|| {
            let m = crate::config::get_airllm_model();
            if m.is_empty() {
                None
            } else {
                Some(m)
            }
        });
    if let Some(m) = model {
        cmd.arg("--model").arg(m);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start AirLLM server: {e}"))?;
    let stdout = child.stdout.take().ok_or("No stdout from AirLLM server.")?;
    let stderr = child.stderr.take().ok_or("No stderr from AirLLM server.")?;
    spawn_reader(stdout, false);
    spawn_reader(stderr, true);
    {
        let mut guard = AIRLLM_CHILD.lock().unwrap();
        *guard = Some(child);
    }
    Ok(json!({
        "started": true,
        "port": PORT,
        "status": status(),
    }))
}

/// Eagerly downloads + shards + loads a model ("install"). Starts the server
/// first if it is not running. The model is loaded layer-by-layer, so on slow
/// machines this can take a long time.
pub async fn install(model: &str) -> Result<Value, String> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err("No model id given. Enter a Hugging Face id (e.g. Qwen/Qwen2.5-0.5B).".to_string());
    }
    if !tcp_probe() {
        start(Some(model.clone()))?;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3600))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("{}/v1/models", base_url()))
        .header("X-Model-Id", model.clone())
        .send()
        .await
        .map_err(|e| format!("[AirLLM] install request error: {e}"))?;
    let status = resp.status();
    let data: Value = resp
        .json()
        .await
        .unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "[AirLLM] install failed (HTTP {status}): {}",
            data["error"]["message"].as_str().unwrap_or("unknown error")
        ));
    }
    Ok(json!({
        "ok": data["loaded"].as_bool().unwrap_or(false),
        "model": data["model"].as_str().unwrap_or(&model),
        "device": data["device"].as_str().unwrap_or(""),
    }))
}

/// Stops the AirLLM server we own.
pub fn stop() -> Result<Value, String> {
    let mut guard = AIRLLM_CHILD.lock().unwrap();
    if let Some(mut c) = guard.take() {
        let _ = c.kill();
        let _ = c.wait();
        READY.store(false, Ordering::Relaxed);
        Ok(json!({ "stopped": true }))
    } else {
        Ok(json!({ "stopped": false, "already_stopped": true }))
    }
}
