//! Local HTTP dashboard for remote control from any device on the LAN,
//! mirroring the Python FastAPI server (plain HTTP, app-level security).
//!
//! Endpoints:
//!   GET  /              -> control panel HTML
//!   GET  /api/status    -> assistant state (voice, keys, os)
//!   POST /api/chat      -> send a chat message (JSON: {"text": "..."})
//!   POST /api/action    -> run an action/tool (JSON: {"name": "...", "args": {...}})
//!   GET  /api/models    -> available local models

use serde_json::{json, Value};
use std::sync::Mutex;
use std::thread;

pub const PORT: u16 = 8000;
static LAST_STATUS: Mutex<(String, bool)> = Mutex::new((String::new(), false));

pub fn set_status_line(line: String, ok: bool) {
    if let Ok(mut g) = LAST_STATUS.lock() {
        *g = (line, ok);
    }
}

const HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Ranveer Remote</title>
<style>
:root{--bg:#0f1115;--panel:#161a22;--panel2:#1c212c;--line:#262c3a;--text:#e8ecf3;--muted:#8b95a7;--acc:#5b8cff}
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:"Segoe UI",system-ui,sans-serif;background:var(--bg);color:var(--text);height:100vh;display:flex}
main{flex:1;margin:auto;max-width:760px;padding:24px;display:flex;flex-direction:column;gap:16px}
h1{font-size:22px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:16px}
label{display:block;font-size:12px;color:var(--muted);margin-bottom:6px}
input,textarea{width:100%;background:var(--panel2);border:1px solid var(--line);color:var(--text);border-radius:8px;padding:10px 12px;font-size:14px}
textarea{min-height:90px;resize:vertical}
.row{display:flex;gap:10px}
button{background:linear-gradient(135deg,#5b8cff,#7c5bff);border:none;color:#fff;border-radius:8px;padding:10px 18px;font-size:14px;cursor:pointer}
#status{font-size:12px;color:var(--muted)}
#status.ok{color:#3ddc84}
#log{font-size:13px;color:var(--muted);white-space:pre-wrap;line-height:1.5}
</style>
</head>
<body>
<main>
<h1>Ranveer Remote</h1>
<div class="card">
  <span id="status">Connecting…</span>
</div>
<div class="card">
  <label for="msg">Send a message to the assistant</label>
  <textarea id="msg" placeholder="e.g. open chrome and search for launcher…"></textarea>
  <div class="row" style="margin-top:10px">
    <button id="send">Send</button>
  </div>
</div>
<div class="card">
  <label>Assistant reply</label>
  <div id="log">—</div>
</div>
</main>
<script>
const statusEl=document.getElementById('status');
const logEl=document.getElementById('log');
async function refresh(){
  try{
    const r=await fetch('/api/status');
    const d=await r.json();
    statusEl.textContent=(d.voice.running?'Voice: ON':'Voice: off')+' · OS: '+d.os+' · AI: '+d.ai_engine+(d.ollama_running?' (Ollama online)':' (Ollama offline)');
    statusEl.className=d.ollama_running?'ok':'';
  }catch(e){statusEl.textContent='Offline: '+e}
}
document.getElementById('send').addEventListener('click',async()=>{
  const msg=document.getElementById('msg').value.trim();
  if(!msg)return;
  logEl.textContent='Thinking…';
  try{
    const r=await fetch('/api/chat',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({text:msg})});
    const d=await r.json();
    logEl.textContent=d.reply||d.error||'No reply';
  }catch(e){logEl.textContent='Error: '+e}
});
setInterval(refresh,3000);refresh();
</script>
</body>
</html>"#;

/// Spawns the dashboard HTTP server on a background thread.
pub fn start() {
    let server = match tiny_http::Server::http(format!("0.0.0.0:{PORT}")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[Dashboard] Could not bind port {PORT}: {e}");
            return;
        }
    };
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().ok();
        println!("[Dashboard] Listening on http://0.0.0.0:{PORT}");
        for mut request in server.incoming_requests() {
            let url = request.url().to_string();
            let method = request.method().to_string();
            let mut body = String::new();
            let mut body_bytes: Vec<u8> = Vec::new();
            if method == "POST" {
                let _ = request.as_reader().read_to_end(&mut body_bytes);
                body = String::from_utf8_lossy(&body_bytes).to_string();
            }
            let (status, content, ctype) = handle(&url, &method, &body, &rt);
            let ctype_header = match tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes()) {
                Ok(h) => h,
                Err(_) => continue,
            };
            let cors_header = match tiny_http::Header::from_bytes(
                &b"Access-Control-Allow-Origin"[..],
                b"*",
            ) {
                Ok(h) => h,
                Err(_) => continue,
            };
            let _ = request.respond(
                tiny_http::Response::from_string(content)
                    .with_status_code(status)
                    .with_header(ctype_header)
                    .with_header(cors_header),
            );
        }
    });
}

fn handle(url: &str, method: &str, body: &str, rt: &Option<tokio::runtime::Runtime>) -> (u16, String, String) {
    let path = url.split('?').next().unwrap_or("/");
    match (method, path) {
        ("GET", "/") => (200, HTML.to_string(), "text/html".to_string()),
        ("GET", "/api/status") => {
            let st = LAST_STATUS
                .lock()
                .map(|g| g.clone())
                .unwrap_or((String::new(), false));
            let os = crate::config::get_os();
            let ai = crate::ai::status();
            let reply = json!({
                "os": os,
                "ai_engine": ai["engine"].as_str().unwrap_or("AI"),
                "ai_provider": ai["provider"].as_str().unwrap_or(""),
                "ai_model": ai["model"].as_str().unwrap_or(""),
                "ai_online": ai["online"].as_bool().unwrap_or(false),
                "ollama_running": crate::ollama::is_running(),
                "voice": {
                    "running": crate::voice::is_running(),
                    "speaking": crate::voice::is_speaking(),
                },
                "status": st.0,
                "status_ok": st.1,
            });
            (200, reply.to_string(), "application/json".to_string())
        }
        ("POST", "/api/chat") => {
            let text = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            if text.is_empty() {
                return (400, json!({"error": "No text"}).to_string(), "application/json".to_string());
            }
            if let Some(rt) = rt {
                let reply = rt.block_on(async {
                    let mut orch = crate::orchestrator::RanveerOrchestrator::new();
                    orch.reply(&text).await
                });
                (200, json!({"reply": reply}).to_string(), "application/json".to_string())
            } else {
                (500, json!({"error": "No runtime"}).to_string(), "application/json".to_string())
            }
        }
        ("POST", "/api/action") => {
            let v = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let args = v.get("args").cloned().unwrap_or(json!({}));
            if let Some(rt) = rt {
                let result = rt.block_on(async {
                    let orch = crate::orchestrator::RanveerOrchestrator::new();
                    orch.execute_tool(&name, &args).await
                });
                (200, json!({"result": result}).to_string(), "application/json".to_string())
            } else {
                (500, json!({"error": "No runtime"}).to_string(), "application/json".to_string())
            }
        }
        ("GET", "/api/models") => {
            let models = crate::or_client::client().available_models();
            (200, json!({"models": models}).to_string(), "application/json".to_string())
        }
        _ => (404, json!({"error": "Not found"}).to_string(), "application/json".to_string()),
    }
}