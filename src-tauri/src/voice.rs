//! Voice conversation using Windows built-in Speech Recognition.
//!
//! This is fully offline and far more reliable than the cpal audio path on this
//! machine, where the default mic config is rejected by cpal. Instead we:
//!   - run a PowerShell System.Speech.SpeechRecognitionEngine (offline, en-US)
//!   - each recognized utterance is printed as `TEXT:<text>` on stdout
//!   - Rust forwards it into the normal chat orchestrator
//!   - the reply is spoken aloud with the SAPI TTS voice

use once_cell::sync::Lazy;
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Emitter;

/// Seconds after an accepted command during which follow-up utterances are
/// processed without needing the wake word again (a conversation window).
const CONV_WINDOW_SECS: u64 = 15;

/// When true (default), always-listening only acts on utterances containing the
/// wake word, or on follow-ups inside the conversation window. Toggleable so a
/// user can run fully open-mic if they prefer.
static WAKE_REQUIRED: AtomicBool = AtomicBool::new(true);

/// True while the current session was started by push-to-talk (or the mic
/// button). Explicit user action → every utterance is processed, no wake word.
static PTT_SESSION: AtomicBool = AtomicBool::new(false);

/// Timestamp of the last accepted interaction (opens the conversation window).
static LAST_INTERACTION: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

pub fn set_wake_required(required: bool) {
    WAKE_REQUIRED.store(required, Ordering::Relaxed);
}

pub fn wake_required() -> bool {
    WAKE_REQUIRED.load(Ordering::Relaxed)
}

fn note_interaction() {
    if let Ok(mut g) = LAST_INTERACTION.lock() {
        *g = Some(Instant::now());
    }
}

enum WakeDecision {
    /// Run this (already wake-word-stripped) text through the assistant.
    Process(String),
    /// Wake word alone ("Ranveer") — acknowledge and open the window.
    Acknowledge,
    /// No wake word and not in a conversation window — ignore.
    Ignore,
}

/// Removes leading filler/wake words so "Ranveer, open notepad" -> "open notepad".
fn strip_wakeword(text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    while let Some(first) = words.first() {
        let w: String = first
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if matches!(
            w.as_str(),
            "hey" | "hi" | "hello" | "ok" | "okay" | "yo" | "ranveer" | "ranbir" | "runveer"
        ) {
            words.remove(0);
        } else {
            break;
        }
    }
    words.join(" ").trim().to_string()
}

/// Decides whether a recognized utterance should be acted on.
fn wake_gate(text: &str) -> WakeDecision {
    if PTT_SESSION.load(Ordering::Relaxed) || !WAKE_REQUIRED.load(Ordering::Relaxed) {
        return WakeDecision::Process(text.to_string());
    }
    let has_wake = crate::llm::wakeword_detected(text);
    // While the assistant is speaking, require an explicit wake word so its own
    // TTS bleeding into the mic can't self-trigger a follow-up.
    if VOICE.speaking.load(Ordering::Relaxed) && !has_wake {
        return WakeDecision::Ignore;
    }
    if has_wake {
        let clean = strip_wakeword(text);
        return if clean.is_empty() {
            WakeDecision::Acknowledge
        } else {
            WakeDecision::Process(clean)
        };
    }
    let in_window = LAST_INTERACTION
        .lock()
        .ok()
        .and_then(|g| *g)
        .map(|t| t.elapsed() < Duration::from_secs(CONV_WINDOW_SECS))
        .unwrap_or(false);
    if in_window {
        WakeDecision::Process(text.to_string())
    } else {
        WakeDecision::Ignore
    }
}

pub struct VoiceState {
    pub running: AtomicBool,
    pub speaking: AtomicBool,
}

impl VoiceState {
    pub fn new() -> Self {
        VoiceState {
            running: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
        }
    }
}

pub static VOICE: Lazy<VoiceState> = Lazy::new(VoiceState::new);

static SR_CHILD: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));

pub fn is_running() -> bool {
    VOICE.running.load(Ordering::Relaxed)
}

pub fn is_speaking() -> bool {
    VOICE.speaking.load(Ordering::Relaxed)
}

fn sr_script() -> String {
    // Continuous dictation via the synchronous Recognize() loop. This is far
    // more reliable than RecognizeAsync + Register-ObjectEvent in a script:
    // the event pump can stall and drop utterances, while Recognize() blocks
    // until something is heard and returns it directly. stderr is surfaced so
    // setup errors are visible.
    // RANVEER_VOICE_INPUT overrides the mic with a wave file (used by tests).
    r#"$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Speech
try {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumeratorComObject { }
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceEnumerator {
  int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
  int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
  int GetDevice(string id, out IMMDevice device);
  int RegisterEndpointNotificationCallback(object client);
  int UnregisterEndpointNotificationCallback(object client);
}
[Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceCollection {
  int GetCount(out int count);
  int Item(int index, out IMMDevice device);
}
[Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDevice {
  int Activate(ref Guid id, int clsCtx, IntPtr activationParams, out object iface);
  int OpenPropertyStore(int access, out IPropertyStore store);
  int GetId(out string id);
  int GetState(out int state);
}
[Guid("886d8eeb-8cf2-4446-8d02-cdba1dbdcf99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IPropertyStore {
  int GetCount(out int count);
  int GetAt(int index, out PropertyKey key);
  int GetValue(ref PropertyKey key, out PropVariant value);
  int SetValue(ref PropertyKey key, ref PropVariant value);
  int Commit();
}
[StructLayout(LayoutKind.Sequential)] struct PropertyKey { public Guid fmtid; public int pid; }
[StructLayout(LayoutKind.Sequential)] struct PropVariant {
  public short vt; public short wReserved1; public short wReserved2; public short wReserved3; public IntPtr value;
}
public class MicInfo {
  static readonly Guid PKEY_FriendlyName = new Guid("{a45c254e-df1c-4efd-8020-67d146a850e0}");
  static string PropName(IPropertyStore s) {
    int n; s.GetCount(out n);
    for (int i = 0; i < n; i++) {
      PropertyKey k; s.GetAt(i, out k);
      if (k.fmtid == PKEY_FriendlyName && k.pid == 14) {
        PropVariant v = new PropVariant(); s.GetValue(ref k, out v);
        if (v.vt == 31) return Marshal.PtrToStringUni(v.value);
      }
    }
    return "";
  }
  public static string DefaultCaptureName() {
    var e = (IMMDeviceEnumerator)new MMDeviceEnumeratorComObject();
    IMMDevice d;
    e.GetDefaultAudioEndpoint(1, 0, out d);
    IPropertyStore s; d.OpenPropertyStore(0, out s);
    return PropName(s);
  }
}
'@ -ErrorAction Stop
  [Console]::Error.WriteLine("VOICE_DEVICE:" + [MicInfo]::DefaultCaptureName())
} catch { }
try {
  $r = New-Object System.Speech.Recognition.SpeechRecognitionEngine
} catch {
  [Console]::Error.WriteLine("VOICE_ERR: could not create recognizer: $_")
  exit 1
}
try {
  $override = $env:RANVEER_VOICE_INPUT
  if ($override) {
    $r.SetInputToWaveFile($override)
  } else {
    $r.SetInputToDefaultAudioDevice()
  }
} catch {
  [Console]::Error.WriteLine("VOICE_ERR: no audio input: $_")
  exit 1
}
$g = New-Object System.Speech.Recognition.DictationGrammar
$r.LoadGrammar($g)
[Console]::Error.WriteLine("VOICE_OK: recognizer listening")
while ($true) {
  try {
    $res = $r.Recognize()
  } catch {
    [Console]::Error.WriteLine("VOICE_ERR: recognize failed: $_")
    Start-Sleep -Milliseconds 500
    continue
  }
  if ($null -ne $res) {
    [Console]::Out.WriteLine("TEXT:" + $res.Text)
    [Console]::Out.Flush()
  }
}
"#
    .to_string()
}

/// Writes the recognizer script to temp and spawns PowerShell with piped
/// stdout + stderr. Returns (child, stdout-pipe, stderr-pipe). Caller owns all.
fn spawn_sr() -> Result<(Child, ChildStdout, ChildStderr), String> {
    let script_path = std::env::temp_dir().join(format!("ranveer_sr_{}.ps1", std::process::id()));
    std::fs::write(&script_path, sr_script()).map_err(|e| e.to_string())?;

    let mut child = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Voice process stdout unavailable.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Voice process stderr unavailable.".to_string())?;
    Ok((child, stdout, stderr))
}

/// Starts the voice session: spawns the Windows speech-recognition process and
/// feeds recognized utterances into the chat orchestrator. Returns immediately;
/// the session keeps running until `stop()` is called.
///
/// `ptt` = true for hold-to-talk / mic button sessions: every utterance is
/// processed directly without requiring the wake word.
pub async fn start(app: tauri::AppHandle, ptt: bool) -> Result<String, String> {
    if VOICE.running.swap(true, Ordering::Relaxed) {
        return Ok("Voice session is already running.".to_string());
    }
    PTT_SESSION.store(ptt, Ordering::Relaxed);

    // Defensive cleanup: kill any stale recognizer from a previous session so it
    // doesn't keep holding the microphone.
    if let Ok(mut guard) = SR_CHILD.lock() {
        if let Some(mut old) = guard.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
    }

    let (child, stdout, stderr) = spawn_sr()?;

    if let Ok(mut guard) = SR_CHILD.lock() {
        *guard = Some(child);
    }

    println!("[Voice] Windows speech recognition started.");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, String)>(64);

    // stdout → recognized utterances
    let tx_out = tx.clone();
    let reader = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                let line = line.trim();
                if let Some(text) = line.strip_prefix("TEXT:") {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        let _ = tx_out.blocking_send(("text".to_string(), text));
                    }
                }
            }
        }
    });

    // stderr → VOICE_OK / VOICE_ERR / VOICE_DEVICE status lines (surfaced in UI)
    let tx_err = tx.clone();
    let err_reader = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                let line = line.trim().to_string();
                if line.starts_with("VOICE_") {
                    let _ = tx_err.blocking_send(("status".to_string(), line));
                }
            }
        }
    });

    // Async task: for each recognized utterance, surface it in the live
    // transcript, then run it through the chat pipeline. Status lines from the
    // recognizer (VOICE_OK / VOICE_DEVICE / VOICE_ERR) are shown too so failures
    // are visible instead of silent.
    let app_ctx = app.clone();
    let speaker_app = app.clone();
    let process_task = tokio::spawn(async move {
        while let Some((kind, text)) = rx.recv().await {
            if kind == "text" && !VOICE.running.load(Ordering::Relaxed) {
                break;
            }
            if kind == "status" {
                let line = text.clone();
                if line.starts_with("VOICE_ERR:") {
                    let err = line.trim_start_matches("VOICE_ERR:").trim().to_string();
                    let _ = app_ctx.emit(
                        "voice-transcript",
                        serde_json::json!({"role": "system", "text": format!("Voice error: {err}")}),
                    );
                } else if line.starts_with("VOICE_DEVICE:") {
                    let dev = line.trim_start_matches("VOICE_DEVICE:").trim().to_string();
                    let _ = app_ctx.emit(
                        "voice-transcript",
                        serde_json::json!({"role": "system", "text": format!("Voice ready — listening on: {dev}")}),
                    );
                } else if line.starts_with("VOICE_OK") {
                    let _ = app_ctx.emit(
                        "voice-transcript",
                        serde_json::json!({"role": "system", "text": "Voice ready — speak now"}),
                    );
                }
                continue;
            }
            let _ = app_ctx.emit(
                "voice-transcript",
                serde_json::json!({"role": "user", "text": text}),
            );
            match wake_gate(&text) {
                WakeDecision::Ignore => {
                    println!("[Voice] ignored (no wake word): {text}");
                    let _ = app_ctx.emit(
                        "voice-transcript",
                        serde_json::json!({"role": "system", "text": "…"}),
                    );
                }
                WakeDecision::Acknowledge => {
                    note_interaction();
                    let _ = app_ctx.emit(
                        "voice-transcript",
                        serde_json::json!({"role": "system", "text": "Yes, sir?"}),
                    );
                    crate::speak_text("Yes, sir?");
                }
                WakeDecision::Process(clean) => {
                    note_interaction();
                    crate::handle_voice_text(app_ctx.clone(), &clean).await;
                    note_interaction();
                }
            }
        }
        let _ = speaker_app.emit(
            "voice-transcript",
            serde_json::json!({"role": "system", "text": "Voice session ended."}),
        );
    });

    // Keep handles alive for the duration of the session.
    let _ = (reader, err_reader, process_task);
    Ok("Voice session started (Windows speech recognition).".to_string())
}

/// Stops the running voice session.
pub fn stop() -> String {
    VOICE.running.store(false, Ordering::Relaxed);
    VOICE.speaking.store(false, Ordering::Relaxed);
    PTT_SESSION.store(false, Ordering::Relaxed);
    if let Ok(mut guard) = SR_CHILD.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    "Voice session stopped.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead as _;

    /// Generates a spoken WAV via SAPI TTS, then feeds it through the app's
    /// real spawn → stdout → line-parse path (RANVEER_VOICE_INPUT override).
    /// Proves recognition + piping work end to end without needing a mic.
    #[test]
    fn recognizer_pipeline_with_wav() {
        let wav = std::env::temp_dir().join("ranveer_test_speech.wav");
        let wav_str = wav.to_string_lossy().replace('\\', "/");
        let gen = format!(
            r#"Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$s.SetOutputToWaveFile("{wav_str}")
$s.Speak("Hello Ranveer open notepad")
$s.Dispose()"#
        );
        let gen_path = std::env::temp_dir().join("ranveer_gen_wav.ps1");
        std::fs::write(&gen_path, gen).expect("write gen script");
        let status = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&gen_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run gen script");
        assert!(status.success(), "TTS wav generation failed");
        assert!(wav.exists(), "wav not created");

        std::env::set_var("RANVEER_VOICE_INPUT", &wav);

        let (mut child, stdout, stderr) = spawn_sr().expect("spawn recognizer");
        // stderr carries VOICE_ERR lines — surface anything it says.
        let err_reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                if let Ok(line) = line {
                    let line = line.trim().to_string();
                    println!("[test] sr status: {line}");
                }
            }
        });
        let reader = BufReader::new(stdout);
        let mut found: Option<String> = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        for line in reader.lines() {
            if let Ok(line) = line {
                let line = line.trim().to_string();
                println!("[test] sr line: {line}");
                if let Some(text) = line.strip_prefix("TEXT:") {
                    found = Some(text.to_string());
                    break;
                }
            }
            if std::time::Instant::now() > deadline {
                break;
            }
        }
        let _ = child.kill();
        let _ = err_reader.join();
        std::env::remove_var("RANVEER_VOICE_INPUT");
        match found {
            Some(t) => println!("[test] RECOGNIZED: {t}"),
            None => panic!("no TEXT line received from recognizer within 60s"),
        }
    }
}