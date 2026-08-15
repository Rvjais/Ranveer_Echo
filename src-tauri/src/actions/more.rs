use serde_json::Value;
use std::process::Command;

fn _powershell(script: &str) -> String {
    #[cfg(windows)]
    {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-STA", "-Command", script])
            .output();
        match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if stdout.is_empty() {
                    stderr
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
        "computer_control is only supported on Windows.".to_string()
    }
}

fn _open_url(url: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_| true)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open").arg(url).spawn().map(|_| true).unwrap_or(false)
    }
}

fn _clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Sends a message by opening a platform deep-link in the default browser.
/// WhatsApp uses wa.me links; Telegram/Instagram fall back to their web clients.
pub fn send_message(args: Value) -> String {
    let receiver = args.get("receiver").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let text = args
        .get("message_text")
        .or_else(|| args.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let platform = args.get("platform").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    let receiver = _clean(&receiver);

    if receiver.is_empty() {
        return "No recipient provided for send_message.".to_string();
    }

    let encoded_text: String = text
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('\n', "%0A");

    match platform.as_str() {
        "whatsapp" | "wa" | "" => {
            let number: String = receiver.chars().filter(|c| c.is_ascii_digit()).collect();
            // wa.me needs a phone number; a contact *name* can't be dialed, so
            // fall back to opening WhatsApp with the message ready to send.
            let url = if number.len() >= 7 {
                format!("https://wa.me/{number}?text={encoded_text}")
            } else {
                format!("https://wa.me/?text={encoded_text}")
            };
            if _open_url(&url) {
                format!("Opened WhatsApp chat for {receiver} with the message.")
            } else {
                "Could not open WhatsApp.".to_string()
            }
        }
        "telegram" | "tg" => {
            let handle = receiver.trim_start_matches('@');
            let url = format!("https://t.me/{handle}?text={encoded_text}");
            if _open_url(&url) {
                format!("Opened Telegram chat for {receiver} with the message.")
            } else {
                "Could not open Telegram.".to_string()
            }
        }
        "instagram" | "ig" | "insta" => {
            let url = format!("https://www.instagram.com/direct/new/?text={encoded_text}");
            if _open_url(&url) {
                format!("Opened Instagram Direct for {receiver}.")
            } else {
                "Could not open Instagram.".to_string()
            }
        }
        other => {
            let url = format!("https://wa.me/?text={encoded_text}");
            if _open_url(&url) {
                format!("Platform '{other}' not directly supported; opened WhatsApp fallback.")
            } else {
                "Could not open a messaging platform.".to_string()
            }
        }
    }
}

/// Finds a YouTube video by opening the search page, or extracts the video id
/// from a URL and returns it with the watch link.
pub fn youtube_video(args: Value) -> String {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("search").to_lowercase();
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();

    match action.as_str() {
        "watch" | "open" if !url.is_empty() => {
            if _open_url(&url) {
                format!("Opened {url}.")
            } else {
                "Could not open the video.".to_string()
            }
        }
        "search" | "play" | "" if !query.is_empty() => {
            let q: String = query.split_whitespace().collect::<Vec<_>>().join("+");
            let search_url = format!("https://www.youtube.com/results?search_query={q}");
            if _open_url(&search_url) {
                format!("Searched YouTube for {query}.")
            } else {
                "Could not open YouTube.".to_string()
            }
        }
        "id" | "extract" if !url.is_empty() => {
            let vid = url.split("v=").nth(1).and_then(|s| s.split('&').next());
            match vid {
                Some(v) if !v.is_empty() => {
                    format!("Video id: {v} — https://youtu.be/{v}")
                }
                _ => "No video id found in that URL.".to_string(),
            }
        }
        _ => "Please provide a query or a video URL for youtube_video.".to_string(),
    }
}

/// Searches flights by opening Google Flights with origin/destination/date.
pub fn flight_finder(args: Value) -> String {
    let origin = args.get("origin").and_then(|v| v.as_str()).unwrap_or("").to_uppercase();
    let destination = args
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_uppercase();
    let date = args.get("date").and_then(|v| v.as_str()).unwrap_or("").to_string();

    if origin.is_empty() || destination.is_empty() {
        return "Both origin and destination are required for flight_finder.".to_string();
    }
    let url = if date.is_empty() {
        format!("https://www.google.com/travel/flights?q=flights+from+{origin}+to+{destination}")
    } else {
        format!(
            "https://www.google.com/travel/flights?q=flights+from+{origin}+to+{destination}+on+{date}"
        )
    };
    if _open_url(&url) {
        format!("Opened flight search: {origin} → {destination} ({date}).")
    } else {
        "Could not open the flight search.".to_string()
    }
}

/// Updates or launches games. Steam games open via the steam:// deep link; if
/// an app_id is provided it launches that specific game.
pub fn game_updater(args: Value) -> String {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list").to_lowercase();
    let platform = args
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("steam")
        .to_lowercase();
    let app_id = args.get("app_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let game = args.get("game_name").and_then(|v| v.as_str()).unwrap_or("").to_string();

    match action.as_str() {
        "update" => {
            if platform == "epic" {
                if _open_url("com.epicgames.launcher://") {
                    "Opened Epic Games Launcher to update games.".to_string()
                } else {
                    "Could not open Epic Games Launcher.".to_string()
                }
            } else if _open_url("steam://open/library") {
                "Opened Steam library — downloads and updates run there.".to_string()
            } else {
                "Could not open Steam.".to_string()
            }
        }
        "install" | "launch" => {
            if !app_id.is_empty() {
                let url = format!("steam://launch/{app_id}");
                if _open_url(&url) {
                    format!("Launching Steam app {app_id} ({game}).")
                } else {
                    "Could not launch the game via Steam.".to_string()
                }
            } else if !game.is_empty() {
                let url = format!("https://store.steampowered.com/search/?term={}", game.replace(' ', "+"));
                if _open_url(&url) {
                    format!("Opened Steam store search for {game}.")
                } else {
                    "Could not open the Steam store.".to_string()
                }
            } else {
                "Provide an app_id or game_name to install/launch.".to_string()
            }
        }
        "list" => {
            let home = dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let steam = std::path::Path::new(&home)
                .join("AppData")
                .join("Roaming")
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Steam");
            let mut names = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&steam) {
                for e in rd.flatten() {
                    names.push(e.file_name().to_string_lossy().to_string());
                }
            }
            if names.is_empty() {
                "No Steam shortcuts found. Open the library to see your games.".to_string()
            } else {
                format!("Steam shortcuts:\n{}", names.join("\n"))
            }
        }
        _ => "Game updater: use action update | install | launch | list.".to_string(),
    }
}

/// Types text into the focused field using Windows SendKeys.
/// Strings with spaces/symbols are copied to the clipboard and pasted for
/// reliability, mimicking the Python smart_type.
pub fn computer_control(args: Value) -> String {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    match action.as_str() {
        "type" | "smart_type" | "typewrite" => {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() {
                return "No text provided to type.".to_string();
            }
            if text.len() > 20 {
                let escaped = text.replace('\'', "''");
                let r = _powershell(&format!(
                    "$t='{escaped}'; Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::SetText($t); Start-Sleep -Milliseconds 100; [System.Windows.Forms.SendKeys]::SendWait('^v')"
                ));
                format!("Smart-typed (clipboard paste): {text} ({r})")
            } else {
                let a = text.replace('+', "{+}").replace('^', "{^}").replace('%', "{%}");
                let r = _powershell(&format!(
                    "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{a}')"
                ));
                format!("Typed: {text} ({r})")
            }
        }
        "press" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
            if key.is_empty() {
                return "No key provided to press.".to_string();
            }
            let key_lower = key.to_lowercase();
            let send = match key_lower.as_str() {
                "enter" => "{ENTER}",
                "tab" => "{TAB}",
                "esc" | "escape" => "{ESC}",
                "space" => " ",
                "up" => "{UP}",
                "down" => "{DOWN}",
                "left" => "{LEFT}",
                "right" => "{RIGHT}",
                "backspace" => "{BACKSPACE}",
                "delete" | "del" => "{DELETE}",
                "home" => "{HOME}",
                "end" => "{END}",
                k if k.len() == 1 => return format!("Pressed: {key} via direct char not sent; use text instead."),
                _ => return format!("Pressed: unsupported key {key}"),
            };
            let r = _powershell(&format!(
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{send}')"
            ));
            format!("Pressed: {key} ({r})")
        }
        "hotkey" | "keys" | "combo" => {
            let keys = args.get("keys").and_then(|v| v.as_str()).unwrap_or("");
            if keys.is_empty() {
                return "No hotkey provided (e.g. ctrl+c).".to_string();
            }
            let parts: Vec<&str> = keys.split('+').map(|k| k.trim()).collect();
            let has_win = parts.iter().any(|k| k.eq_ignore_ascii_case("win"));
            let mapped = parts
                .iter()
                .filter(|k| !k.eq_ignore_ascii_case("win"))
                .map(|k| {
                    let kt = k.to_lowercase();
                    match kt.as_str() {
                        "ctrl" => "^".to_string(),
                        "shift" => "+".to_string(),
                        "alt" => "%".to_string(),
                        other => other.to_string(),
                    }
                })
                .collect::<Vec<_>>()
                .concat();
            // SendKeys can't send the Windows key; hold it down with keybd_event
            // around the SendKeys payload instead.
            let r = if has_win {
                _powershell(&format!(
                    "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class K{{[DllImport(\"user32.dll\")]public static extern void keybd_event(byte k,byte s,uint f,int e);}}'; Add-Type -AssemblyName System.Windows.Forms; [K]::keybd_event(0x5B,0,0,0); [System.Windows.Forms.SendKeys]::SendWait('{mapped}'); [K]::keybd_event(0x5B,0,2,0)"
                ))
            } else {
                _powershell(&format!(
                    "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{mapped}')"
                ))
            };
            format!("Hotkey: {keys} ({r})")
        }
        "scroll" => {
            let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("down");
            let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3);
            let script = if direction == "up" {
                format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{{UP {amount}}}')")
            } else {
                format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{{DOWN {amount}}}')")
            };
            let r = _powershell(&script);
            format!("Scrolled {direction} {amount} ({r})")
        }
        "screenshot" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("desktop")
                .to_string();
            let dest = if path == "desktop" {
                let d = dirs::desktop_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".to_string());
                format!("{}\\ranveer_screenshot.png", d.trim_end_matches('\\'))
            } else {
                path
            };
            let script = format!(
                "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); $bmp.Save('{dest}'); $g.Dispose(); $bmp.Dispose()"
            );
            let r = _powershell(&script);
            format!("Screenshot saved to {dest} ({r})")
        }
        "click" => {
            let x = args.get("x").and_then(|v| v.as_i64());
            let y = args.get("y").and_then(|v| v.as_i64());
            let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left");
            if let (Some(x), Some(y)) = (x, y) {
                let script = format!(
                    "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class M{{[DllImport(\"user32.dll\")]public static extern void SetCursorPos(int x,int y);[DllImport(\"user32.dll\")]public static extern void mouse_event(uint f,uint dx,uint dy,uint d,uint p);}}'; [M]::SetCursorPos({x},{y}); $down=0x0002; $up=0x0004; [M]::mouse_event($down,0,0,0,0); [M]::mouse_event($up,0,0,0,0)"
                );
                let r = _powershell(&script);
                format!("Clicked ({x}, {y}) [{button}] ({r})")
            } else {
                "For 'click' provide x and y coordinates.".to_string()
            }
        }
        "move" => {
            let x = args.get("x").and_then(|v| v.as_i64());
            let y = args.get("y").and_then(|v| v.as_i64());
            if let (Some(x), Some(y)) = (x, y) {
                let r = _powershell(&format!(
                    "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class M{{[DllImport(\"user32.dll\")]public static extern bool SetCursorPos(int x,int y);}}'; [M]::SetCursorPos({x},{y})"
                ));
                format!("Moved mouse to ({x}, {y}) ({r})")
            } else {
                "For 'move' provide x and y coordinates.".to_string()
            }
        }
        "copy" => {
            let r = _powershell(
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^c')",
            );
            format!("Copied the selection to the clipboard ({r}).")
        }
        "paste" => {
            let r = _powershell(
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^v')",
            );
            format!("Pasted from the clipboard ({r}).")
        }
        "get_clipboard" | "read_clipboard" => {
            let r = _powershell("Get-Clipboard");
            if r.is_empty() {
                "The clipboard is empty.".to_string()
            } else {
                format!("Clipboard contents:\n{r}")
            }
        }
        "set_clipboard" => {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() {
                return "No text provided to set on the clipboard.".to_string();
            }
            let escaped = text.replace('\'', "''");
            _powershell(&format!(
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::SetText('{escaped}')"
            ));
            "Copied the text to the clipboard.".to_string()
        }
        "focus_window" | "activate_window" => {
            let title = args
                .get("title")
                .or_else(|| args.get("window"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if title.is_empty() {
                return "Provide a window title to focus.".to_string();
            }
            let escaped = title.replace('\'', "''");
            _powershell(&format!(
                "(New-Object -ComObject WScript.Shell).AppActivate('{escaped}')"
            ));
            format!("Focused the window matching '{title}'.")
        }
        "wait" | "sleep" => {
            let secs = args
                .get("seconds")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
                .clamp(0.0, 30.0);
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            format!("Waited {secs} second(s).")
        }
        "screen_find" | "screen_click" => {
            if action == "screen_find" {
                "screen_find needs an image/template matcher which is not wired into the Rust core yet.".to_string()
            } else {
                "screen_click needs an image/template matcher which is not wired into the Rust core yet.".to_string()
            }
        }
        _ => format!("computer_control: unknown action '{action}'."),
    }
}
