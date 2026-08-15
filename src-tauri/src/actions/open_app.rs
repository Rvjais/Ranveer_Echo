use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;

fn app_aliases() -> HashMap<&'static str, [&'static str; 3]> {
    // [Windows, macOS, Linux]
    let mut m = HashMap::new();
    m.insert("whatsapp", ["WhatsApp", "WhatsApp", "whatsapp"]);
    m.insert("chrome", ["chrome", "Google Chrome", "google-chrome"]);
    m.insert("google chrome", ["chrome", "Google Chrome", "google-chrome"]);
    m.insert("firefox", ["firefox", "Firefox", "firefox"]);
    m.insert("spotify", ["Spotify", "Spotify", "spotify"]);
    m.insert("vscode", ["code", "Visual Studio Code", "code"]);
    m.insert("visual studio code", ["code", "Visual Studio Code", "code"]);
    m.insert("discord", ["Discord", "Discord", "discord"]);
    m.insert("telegram", ["Telegram", "Telegram", "telegram"]);
    m.insert("instagram", ["Instagram", "Instagram", "instagram"]);
    m.insert("tiktok", ["TikTok", "TikTok", "tiktok"]);
    m.insert("notepad", ["notepad.exe", "TextEdit", "gedit"]);
    m.insert("calculator", ["calc.exe", "Calculator", "gnome-calculator"]);
    m.insert("terminal", ["cmd.exe", "Terminal", "gnome-terminal"]);
    m.insert("cmd", ["cmd.exe", "Terminal", "bash"]);
    m.insert("explorer", ["explorer.exe", "Finder", "nautilus"]);
    m.insert("file explorer", ["explorer.exe", "Finder", "nautilus"]);
    m.insert("paint", ["mspaint.exe", "Preview", "gimp"]);
    m.insert("word", ["winword", "Microsoft Word", "libreoffice --writer"]);
    m.insert("excel", ["excel", "Microsoft Excel", "libreoffice --calc"]);
    m.insert("powerpoint", ["powerpnt", "Microsoft PowerPoint", "libreoffice --impress"]);
    m.insert("vlc", ["vlc", "VLC", "vlc"]);
    m.insert("zoom", ["Zoom", "zoom.us", "zoom"]);
    m.insert("slack", ["Slack", "Slack", "slack"]);
    m.insert("steam", ["steam", "Steam", "steam"]);
    m.insert("task manager", ["taskmgr.exe", "Activity Monitor", "gnome-system-monitor"]);
    m.insert("settings", ["ms-settings:", "System Preferences", "gnome-control-center"]);
    m.insert("powershell", ["powershell.exe", "Terminal", "bash"]);
    m.insert("edge", ["msedge", "Microsoft Edge", "microsoft-edge"]);
    m.insert("brave", ["brave", "Brave Browser", "brave-browser"]);
    m.insert("obsidian", ["Obsidian", "Obsidian", "obsidian"]);
    m.insert("notion", ["Notion", "Notion", "notion"]);
    m.insert("blender", ["blender", "Blender", "blender"]);
    m.insert("capcut", ["CapCut", "CapCut", "capcut"]);
    m.insert("postman", ["Postman", "Postman", "postman"]);
    m.insert("figma", ["Figma", "Figma", "figma"]);
    m
}

fn detect_os() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Darwin"
    } else {
        "Linux"
    }
}

fn normalize(raw: &str) -> String {
    let system = detect_os();
    let key = raw.trim().to_lowercase();
    let aliases = app_aliases();
    if let Some(map) = aliases.get(key.as_str()) {
        return match system {
            "Windows" => map[0].to_string(),
            "Darwin" => map[1].to_string(),
            _ => map[2].to_string(),
        };
    }
    for (alias_key, map) in aliases.iter() {
        if key.contains(alias_key) || alias_key.contains(&key) {
            return match system {
                "Windows" => map[0].to_string(),
                "Darwin" => map[1].to_string(),
                _ => map[2].to_string(),
            };
        }
    }
    raw.to_string()
}

fn launch_app(app_name: &str) -> bool {
    #[cfg(windows)]
    {
        // Try to launch directly; if that fails, fall back to `start`
        let result = Command::new("cmd")
            .args(["/C", "start", "", app_name])
            .spawn();
        match result {
            Ok(_) => true,
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app_name;
        false
    }
}

pub fn open_app(parameters: Value) -> String {
    let app_name = parameters
        .get("app_name")
        .or_else(|| parameters.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if app_name.is_empty() {
        return "Please specify which application to open, sir.".to_string();
    }

    let normalized = normalize(&app_name);
    println!("[open_app] Launching: {app_name} -> {normalized}");

    let success = launch_app(&normalized)
        || (normalized != app_name && launch_app(&app_name));

    if success {
        format!("Opened {} successfully, sir.", app_name)
    } else {
        format!(
            "I tried to open {}, sir, but couldn't confirm it launched. It may still be loading or might not be installed.",
            app_name
        )
    }
}