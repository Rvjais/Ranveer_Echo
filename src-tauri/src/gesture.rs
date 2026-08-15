//! Camera + attention monitoring (fully local, no cloud AI).
//!
//! - live camera stream + gesture + face identification via the Python bridge
//!   (`bridge.py stream`), emitted as `camera-frame` Tauri events
//! - one-shot gesture control, face registration and face identification
//!   through the bridge
//! - system attention state (idle time, active window)
//!
//! No Gemini / OpenRouter / any cloud vision is used anywhere.

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// Last face-recognized operator name, updated by `face_identify` / registration.
static CURRENT_OPERATOR: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Child process for the live camera stream bridge.
static CAMERA_CHILD: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));

/// Child process for one-shot gesture control.
pub static GESTURE_CHILD: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));

fn venv_python() -> String {
    #[cfg(windows)]
    {
        let base = crate::config::base_dir();
        let venv = base.join("venv").join("Scripts").join("python.exe");
        if venv.exists() {
            return venv.to_string_lossy().to_string();
        }
        "python".to_string()
    }
    #[cfg(not(windows))]
    {
        "python3".to_string()
    }
}

fn bridge_script() -> String {
    crate::config::base_dir().join("bridge.py").to_string_lossy().to_string()
}

/// Runs a one-shot bridge command synchronously and returns parsed JSON.
fn run_bridge_capture(args: &[&str]) -> Result<Value, String> {
    let py = venv_python();
    let script = bridge_script();
    let out = Command::new(&py)
        .arg(&script)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Could not start Python bridge: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines().rev() {
        if line.trim_start().starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                return Ok(v);
            }
        }
    }
    if !stderr.trim().is_empty() {
        return Err(stderr.trim().to_string());
    }
    Err("Bridge returned no valid output.".to_string())
}

pub fn set_operator(name: Option<String>) {
    if let Ok(mut g) = CURRENT_OPERATOR.lock() {
        *g = name;
    }
}

pub fn current_operator() -> Option<String> {
    CURRENT_OPERATOR.lock().ok().and_then(|g| g.clone())
}

fn _ps(script: &str) -> String {
    #[cfg(windows)]
    {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    String::from_utf8_lossy(&o.stderr).trim().to_string()
                } else {
                    s
                }
            }
            Err(e) => format!("PowerShell error: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = script;
        "Unsupported OS.".to_string()
    }
}

/// System attention state: idle seconds + active window.
pub fn attention_status() -> Value {
    let idle = _ps(
        "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class I{[DllImport(\"user32.dll\")]public static extern bool GetLastInputInfo(ref LASTINPUTINFO p);}public struct LASTINPUTINFO{public uint cbSize;public uint dwTime;}'; $p=New-Object I+LASTINPUTINFO; $p.cbSize=[Runtime.InteropServices.Marshal]::SizeOf($p); [I]::GetLastInputInfo([ref]$p); $tick=[Environment]::TickCount; $idle=($tick - $p.dwTime)/1000; [Math]::Round($idle,1)",
    );
    let idle_secs: f64 = idle.trim().parse().unwrap_or(0.0);
    let window = _ps(
        "Add-Type -TypeDefinition 'using System;using System.Text;using System.Runtime.InteropServices;public class W{[DllImport(\"user32.dll\")]public static extern IntPtr GetForegroundWindow();[DllImport(\"user32.dll\")]public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);}'; $h=[W]::GetForegroundWindow(); $sb=New-Object System.Text.StringBuilder 256; [W]::GetWindowText($h,$sb,256) | Out-Null; $sb.ToString()",
    );
    let active_window = window.trim().to_string();
    json!({
        "idle_seconds": idle_secs,
        "active_window": active_window,
        "away": idle_secs > 300.0,
    })
}

/// Kills a child process if it is still running.
fn kill_child(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Starts one-shot gesture control (moves cursor + click when a hand is seen).
pub fn gesture_start() -> Result<String, String> {
    if GESTURE_CHILD.lock().map(|g| g.is_some()).unwrap_or(false) {
        return Ok("Gesture control is already running, sir.".to_string());
    }
    let py = venv_python();
    let script = bridge_script();
    let child = Command::new(&py)
        .arg(&script)
        .arg("gesture")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start gesture bridge: {e}"))?;
    if let Ok(mut g) = GESTURE_CHILD.lock() {
        *g = Some(child);
    }
    Ok("Gesture control started. Raise your hand to move the cursor, pinch to click. Say 'stop gestures' to end it.".to_string())
}

/// Stops one-shot gesture control.
pub fn gesture_stop() {
    if let Ok(mut g) = GESTURE_CHILD.lock() {
        if let Some(mut child) = g.take() {
            kill_child(&mut child);
        }
    }
}

/// Starts the live camera stream. Frames + gesture + face data are read from the
/// bridge's stdout and emitted as `camera-frame` events to the frontend.
pub fn camera_stream_start(app: tauri::AppHandle) -> Result<String, String> {
    if CAMERA_CHILD.lock().map(|g| g.is_some()).unwrap_or(false) {
        return Ok("Camera feed is already running, sir.".to_string());
    }
    let py = venv_python();
    let script = bridge_script();
    let mut child = Command::new(&py)
        .arg(&script)
        .arg("stream")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start camera bridge: {e}"))?;
    let stdout = child.stdout.take().ok_or("No stdout from camera bridge.")?;
    if let Ok(mut g) = CAMERA_CHILD.lock() {
        *g = Some(child);
    }
    std::thread::spawn(move || {
        use tauri::Emitter;
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if !line.starts_with('{') {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let _ = app.emit("camera-frame", &v);
            }
        }
        if let Ok(mut g) = CAMERA_CHILD.lock() {
            *g = None;
        }
        println!("[Camera] stream ended");
    });
    Ok("Camera feed started, sir. You can now use hand gestures and face identification from the chat page.".to_string())
}

/// Stops the live camera stream.
pub fn camera_stream_stop() {
    if let Ok(mut g) = CAMERA_CHILD.lock() {
        if let Some(mut child) = g.take() {
            kill_child(&mut child);
        }
    }
}

/// Lists registered faces.
pub fn face_list() -> Result<Value, String> {
    run_bridge_capture(&["list"])
}

/// Registers the current camera subject as `name` (collects samples).
pub async fn register_face_async(name: &str) -> Result<Value, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Please provide a name to register.".to_string());
    }
    let result = run_bridge_capture(&["register", &name])?;
    set_operator(Some(name));
    Ok(result)
}

/// Identifies the current camera subject; updates the operator.
pub async fn identify_face_async() -> Result<Value, String> {
    let result = run_bridge_capture(&["identify"])?;
    let name = result
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !name.is_empty() {
        set_operator(Some(name));
    }
    Ok(result)
}
