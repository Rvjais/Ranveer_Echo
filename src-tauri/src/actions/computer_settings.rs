//! Windows system-setting control: volume, brightness, window management,
//! system toggles, media keys and power. Mirrors the Python
//! actions/computer_settings.py surface. Implemented via PowerShell shell-outs
//! (the same convention as actions/more.rs) so no native crates are required.

use serde_json::Value;
use std::process::Command;

/// Runs a PowerShell script and returns trimmed stdout (or stderr if empty).
fn ps(script: &str) -> String {
    #[cfg(windows)]
    {
        match Command::new("powershell")
            .args(["-NoProfile", "-STA", "-Command", script])
            .output()
        {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if stdout.is_empty() {
                    String::from_utf8_lossy(&o.stderr).trim().to_string()
                } else {
                    stdout
                }
            }
            Err(e) => format!("PowerShell error: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = script;
        "computer_settings is only supported on Windows.".to_string()
    }
}

/// Sends N presses of a media/volume virtual key via user32 keybd_event.
/// SendKeys([char]...) cannot synthesize media/volume keys — it just types
/// text characters — so we P/Invoke keybd_event with the real VK codes.
fn send_char(code: u32, times: u32) -> String {
    ps(&format!(
        "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class K{{[DllImport(\"user32.dll\")]public static extern void keybd_event(byte k,byte s,uint f,int e);}}'; 1..{times} | % {{ [K]::keybd_event({code},0,0,0); [K]::keybd_event({code},0,2,0) }}"
    ))
}

const VK_VOL_MUTE: u32 = 173; // VK_VOLUME_MUTE 0xAD
const VK_VOL_DOWN: u32 = 174; // VK_VOLUME_DOWN 0xAE
const VK_VOL_UP: u32 = 175;   // VK_VOLUME_UP   0xAF
const VK_MEDIA_NEXT: u32 = 176;      // VK_MEDIA_NEXT_TRACK 0xB0
const VK_MEDIA_PREV: u32 = 177;      // VK_MEDIA_PREV_TRACK 0xB1
const VK_MEDIA_STOP: u32 = 178;      // VK_MEDIA_STOP       0xB2
const VK_MEDIA_PLAY_PAUSE: u32 = 179; // VK_MEDIA_PLAY_PAUSE 0xB3

const DANGEROUS: [&str; 3] = ["restart", "shutdown", "reboot"];

fn is_confirmed(parameters: &Value) -> bool {
    let raw = parameters
        .get("confirmed")
        .map(|v| match v {
            Value::Bool(b) => b.to_string(),
            other => other.as_str().unwrap_or("").to_string(),
        })
        .unwrap_or_default()
        .to_lowercase();
    matches!(raw.as_str(), "yes" | "true" | "1" | "confirm" | "confirmed")
}

pub fn computer_settings(parameters: Value) -> String {
    let parameters = if parameters.is_object() {
        parameters
    } else {
        serde_json::json!({})
    };

    // Accept an explicit action, or fall back to natural-language detection.
    let mut action = parameters
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let description = parameters
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if action.is_empty() && !description.is_empty() {
        action = detect_action(&description);
    }
    if action.is_empty() {
        return "No setting action or description provided.".to_string();
    }

    // Confirmation gate for destructive power actions.
    if DANGEROUS.contains(&action.as_str()) && !is_confirmed(&parameters) {
        return format!(
            "This will {action} the computer. Call again with confirmed=yes to proceed."
        );
    }

    let value = parameters
        .get("value")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            parameters
                .get("value")
                .and_then(|v| v.as_str())
                .and_then(|s| s.trim().trim_end_matches('%').parse::<i64>().ok())
        });

    match action.as_str() {
        // ---- Volume ----
        "volume_up" | "vol_up" => {
            send_char(VK_VOL_UP, 5);
            "Turned the volume up.".to_string()
        }
        "volume_down" | "vol_down" => {
            send_char(VK_VOL_DOWN, 5);
            "Turned the volume down.".to_string()
        }
        "mute" | "volume_mute" | "toggle_mute" => {
            send_char(VK_VOL_MUTE, 1);
            "Toggled mute.".to_string()
        }
        "unmute" => {
            // Best effort: nudge volume up, which also clears mute on Windows.
            send_char(VK_VOL_UP, 1);
            "Unmuted (nudged the volume).".to_string()
        }
        "volume_set" | "set_volume" => {
            let target = value.unwrap_or(50).clamp(0, 100);
            // Windows volume steps ~2% per key press. Drive to 0, then up to N.
            send_char(VK_VOL_DOWN, 50);
            let ups = (target as u32).div_ceil(2);
            if ups > 0 {
                send_char(VK_VOL_UP, ups);
            }
            format!("Set volume to about {target}%.")
        }

        // ---- Brightness (laptop integrated display via WMI) ----
        "brightness_up" => set_brightness_delta(10),
        "brightness_down" => set_brightness_delta(-10),
        "brightness_set" | "set_brightness" => {
            let target = value.unwrap_or(70).clamp(0, 100);
            ps(&format!(
                "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods).WmiSetBrightness(1,{target})"
            ));
            format!("Set brightness to {target}%.")
        }

        // ---- Window management ----
        "minimize" => show_window(6, "Minimized the active window."),
        "maximize" => show_window(3, "Maximized the active window."),
        "restore" | "unmaximize" => show_window(9, "Restored the active window."),
        "minimize_all" | "show_desktop" => {
            ps("(New-Object -ComObject Shell.Application).ToggleDesktop()");
            "Toggled show desktop.".to_string()
        }
        "close_window" | "close_app" => {
            ps("(New-Object -ComObject WScript.Shell).SendKeys('%{F4}')");
            "Closed the active window.".to_string()
        }
        "switch_window" => {
            ps("(New-Object -ComObject WScript.Shell).SendKeys('%{TAB}')");
            "Switched window.".to_string()
        }
        "snap_left" => snap_window(true),
        "snap_right" => snap_window(false),
        "task_manager" => {
            let _ = Command::new("taskmgr").spawn();
            "Opened Task Manager.".to_string()
        }

        // ---- System toggles ----
        "lock" | "lock_screen" => {
            let _ = Command::new("rundll32.exe")
                .args(["user32.dll,LockWorkStation"])
                .spawn();
            "Locked the screen.".to_string()
        }
        "dark_mode" => set_theme(false),
        "light_mode" => set_theme(true),
        "toggle_theme" | "toggle_dark_mode" => {
            let cur = ps("(Get-ItemProperty -Path 'HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize' -Name AppsUseLightTheme).AppsUseLightTheme");
            // If currently light (1) switch to dark, else light.
            if cur.trim() == "1" {
                set_theme(false)
            } else {
                set_theme(true)
            }
        }
        "open_settings" => {
            let _ = Command::new("cmd").args(["/C", "start", "ms-settings:"]).spawn();
            "Opened Windows Settings.".to_string()
        }
        "file_explorer" | "explorer" => {
            let _ = Command::new("explorer").spawn();
            "Opened File Explorer.".to_string()
        }
        "run_dialog" | "open_run" => {
            ps("(New-Object -ComObject Shell.Application).FileRun()");
            "Opened the Run dialog.".to_string()
        }

        // ---- Media keys ----
        "play_pause" | "play" | "pause" => {
            send_char(VK_MEDIA_PLAY_PAUSE, 1);
            "Toggled play/pause.".to_string()
        }
        "next_track" | "next" => {
            send_char(VK_MEDIA_NEXT, 1);
            "Skipped to the next track.".to_string()
        }
        "prev_track" | "previous" | "prev" => {
            send_char(VK_MEDIA_PREV, 1);
            "Went to the previous track.".to_string()
        }
        "stop_media" | "stop" => {
            send_char(VK_MEDIA_STOP, 1);
            "Stopped media playback.".to_string()
        }

        // ---- Power ----
        "restart" | "reboot" => {
            let _ = Command::new("shutdown").args(["/r", "/t", "0"]).spawn();
            "Restarting the computer now.".to_string()
        }
        "shutdown" => {
            let _ = Command::new("shutdown").args(["/s", "/t", "0"]).spawn();
            "Shutting down the computer now.".to_string()
        }
        "sleep" | "sleep_display" => {
            let _ = Command::new("rundll32.exe")
                .args(["powrprof.dll,SetSuspendState", "0,1,0"])
                .spawn();
            "Putting the computer to sleep.".to_string()
        }

        other => format!("computer_settings: unknown action '{other}'."),
    }
}

/// Adjusts laptop brightness by a delta relative to the current WMI level.
fn set_brightness_delta(delta: i64) -> String {
    let cur = ps("(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness).CurrentBrightness")
        .lines()
        .next()
        .and_then(|l| l.trim().parse::<i64>().ok())
        .unwrap_or(50);
    let target = (cur + delta).clamp(0, 100);
    ps(&format!(
        "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods).WmiSetBrightness(1,{target})"
    ));
    format!("Set brightness to {target}%.")
}

/// Calls ShowWindow on the foreground window. cmd: 3=maximize, 6=minimize, 9=restore.
fn show_window(cmd: i32, ok_msg: &str) -> String {
    let script = format!(
        "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class W{{[DllImport(\"user32.dll\")]public static extern IntPtr GetForegroundWindow();[DllImport(\"user32.dll\")]public static extern bool ShowWindow(IntPtr h,int c);}}'; [W]::ShowWindow([W]::GetForegroundWindow(),{cmd})"
    );
    let r = ps(&script);
    if r.starts_with("PowerShell error") {
        format!("Could not adjust the window ({r}).")
    } else {
        ok_msg.to_string()
    }
}

/// Snaps the active window left or right using the Win+Arrow shortcut.
fn snap_window(left: bool) -> String {
    let arrow = if left { "LEFT" } else { "RIGHT" };
    // keybd_event: LWIN=0x5B, LEFT=0x25, RIGHT=0x27; 0x0002 = KEYUP.
    let vk = if left { 0x25 } else { 0x27 };
    let script = format!(
        "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class K{{[DllImport(\"user32.dll\")]public static extern void keybd_event(byte k,byte s,uint f,int e);}}'; [K]::keybd_event(0x5B,0,0,0); [K]::keybd_event({vk},0,0,0); [K]::keybd_event({vk},0,2,0); [K]::keybd_event(0x5B,0,2,0)"
    );
    ps(&script);
    format!("Snapped the window {}.", arrow.to_lowercase())
}

/// Sets the Windows apps + system theme to light (true) or dark (false).
fn set_theme(light: bool) -> String {
    let v = if light { 1 } else { 0 };
    let base = "HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
    ps(&format!(
        "Set-ItemProperty -Path '{base}' -Name AppsUseLightTheme -Value {v}; Set-ItemProperty -Path '{base}' -Name SystemUsesLightTheme -Value {v}"
    ));
    if light {
        "Switched to light mode.".to_string()
    } else {
        "Switched to dark mode.".to_string()
    }
}

/// Maps a natural-language phrase to a concrete action name.
fn detect_action(desc: &str) -> String {
    let d = desc.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| d.contains(w));

    if has(&["mute"]) && !has(&["unmute"]) {
        return "mute".to_string();
    }
    if has(&["unmute"]) {
        return "unmute".to_string();
    }
    if has(&["volume", "sound", "louder", "quieter"]) {
        if has(&["up", "increase", "louder", "raise"]) {
            return "volume_up".to_string();
        }
        if has(&["down", "decrease", "lower", "quieter", "reduce"]) {
            return "volume_down".to_string();
        }
        if has(&["set", "to "]) {
            return "volume_set".to_string();
        }
    }
    if has(&["brightness", "dim", "brighter"]) {
        if has(&["up", "increase", "brighter", "raise"]) {
            return "brightness_up".to_string();
        }
        if has(&["down", "decrease", "dim", "lower"]) {
            return "brightness_down".to_string();
        }
        if has(&["set", "to "]) {
            return "brightness_set".to_string();
        }
    }
    if has(&["dark mode", "dark theme"]) {
        return "dark_mode".to_string();
    }
    if has(&["light mode", "light theme"]) {
        return "light_mode".to_string();
    }
    if has(&["lock"]) {
        return "lock".to_string();
    }
    if has(&["minimize"]) {
        return "minimize".to_string();
    }
    if has(&["maximize", "maximise"]) {
        return "maximize".to_string();
    }
    if has(&["show desktop", "minimize all"]) {
        return "show_desktop".to_string();
    }
    if has(&["close window", "close this", "close the window"]) {
        return "close_window".to_string();
    }
    if has(&["task manager"]) {
        return "task_manager".to_string();
    }
    if has(&["settings"]) {
        return "open_settings".to_string();
    }
    if has(&["explorer", "file manager"]) {
        return "file_explorer".to_string();
    }
    if has(&["play", "pause"]) {
        return "play_pause".to_string();
    }
    if has(&["next track", "next song", "skip"]) {
        return "next_track".to_string();
    }
    if has(&["previous", "last track", "last song"]) {
        return "prev_track".to_string();
    }
    if has(&["restart", "reboot"]) {
        return "restart".to_string();
    }
    if has(&["shut down", "shutdown", "power off", "turn off the computer"]) {
        return "shutdown".to_string();
    }
    if has(&["sleep", "suspend"]) {
        return "sleep".to_string();
    }
    String::new()
}
