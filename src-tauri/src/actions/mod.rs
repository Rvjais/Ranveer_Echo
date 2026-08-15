pub mod computer_settings;
pub mod desktop;
pub mod file_processor;
pub mod more;
pub mod open_app;
pub mod reminder;
pub mod shell;
pub mod weather;
pub mod web_search;

use serde_json::Value;
use std::collections::HashMap;

pub type ActionFn = fn(Value) -> String;

pub struct ActionRegistry {
    pub actions: HashMap<&'static str, ActionFn>,
}

pub static REGISTRY: once_cell::sync::Lazy<ActionRegistry> = once_cell::sync::Lazy::new(|| {
    let mut actions = HashMap::new();
    actions.insert("open_app", open_app::open_app as ActionFn);
    actions.insert("reminder", reminder::reminder as ActionFn);
    actions.insert("weather_report", weather::weather_action as ActionFn);
    actions.insert("web_search", web_search::web_search as ActionFn);
    actions.insert("shell_command", shell::shell_command as ActionFn);
    actions.insert("desktop_control", desktop::desktop_control as ActionFn);
    ActionRegistry { actions }
});

pub fn execute(name: &str, parameters: Value) -> Result<String, String> {
    REGISTRY
        .actions
        .get(name)
        .map(|f| f(parameters))
        .ok_or_else(|| format!("Unknown action: {name}"))
}

/// Minimal file_controller used by the agent executor and orchestrator.
pub fn agent_executor_file_controller(parameters: &Value) -> Result<String, String> {
    let action = parameters
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut path = parameters
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = parameters
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content = parameters
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Resolve well-known folder shortcuts.
    if let Some(resolved) = resolve_shortcut(&path) {
        path = resolved;
    }

    match action.as_str() {
        "write" | "create_file" => {
            let full = if path.ends_with('\\') || path.ends_with('/') {
                format!("{path}{name}")
            } else {
                format!("{path}\\{name}")
            };
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            std::fs::write(&full, content).map_err(|e| e.to_string())?;
            Ok(format!("Wrote file: {full}"))
        }
        "create_folder" | "mkdir" => {
            let full = if name.is_empty() {
                path.clone()
            } else {
                format!("{}\\{name}", path.trim_end_matches(['\\', '/']))
            };
            std::fs::create_dir_all(&full).map_err(|e| e.to_string())?;
            Ok(format!("Created folder: {full}"))
        }
        "read" => std::fs::read_to_string(&path).map_err(|e| e.to_string()),
        "list" => {
            let mut lines = Vec::new();
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())?.flatten() {
                lines.push(entry.file_name().to_string_lossy().to_string());
            }
            Ok(format!("Files in {path}:\n{}", lines.join("\n")))
        }
        "delete" => {
            let target = std::path::Path::new(&path);
            // Guard against wiping a whole well-known folder from a bare call.
            if target.is_dir() {
                return Err(format!(
                    "Refusing to delete the directory '{path}'. Provide a specific file name."
                ));
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            Ok(format!("Deleted {path}"))
        }
        "find" | "find_files" | "search" => {
            let needle = if !name.is_empty() {
                name.to_lowercase()
            } else {
                parameters
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase()
            };
            if needle.is_empty() {
                return Err("Provide a file name or query to search for.".to_string());
            }
            let mut hits = Vec::new();
            find_files(std::path::Path::new(&path), &needle, &mut hits, 0);
            if hits.is_empty() {
                Ok(format!("No files matching '{needle}' under {path}."))
            } else {
                Ok(format!(
                    "Found {} match(es) for '{needle}':\n{}",
                    hits.len(),
                    hits.into_iter().take(30).collect::<Vec<_>>().join("\n")
                ))
            }
        }
        "disk_usage" | "disk" => {
            #[cfg(windows)]
            {
                let out = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-Command",
                        "Get-PSDrive -PSProvider FileSystem | Select-Object Name,@{N='UsedGB';E={[math]::Round($_.Used/1GB,1)}},@{N='FreeGB';E={[math]::Round($_.Free/1GB,1)}} | Format-Table -AutoSize | Out-String"])
                    .output()
                    .map_err(|e| e.to_string())?;
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            #[cfg(not(windows))]
            {
                Ok("Disk usage is only supported on Windows.".to_string())
            }
        }
        "info" | "get_file_info" | "file_info" => {
            let target = std::path::Path::new(&path);
            let meta = std::fs::metadata(target).map_err(|e| e.to_string())?;
            let kind = if meta.is_dir() { "folder" } else { "file" };
            let size = meta.len();
            let size_str = if size > 1024 * 1024 {
                format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
            } else {
                format!("{:.1} KB", size as f64 / 1024.0)
            };
            Ok(format!("{path}\n  Type: {kind}\n  Size: {size_str}\n  Read-only: {}", meta.permissions().readonly()))
        }
        _ => Ok(format!("No action specified for {path}")),
    }
}

/// Resolves a shortcut like "desktop"/"downloads"/"documents" to a full path.
fn resolve_shortcut(path: &str) -> Option<String> {
    let dir = match path.trim().to_lowercase().as_str() {
        "desktop" => dirs::desktop_dir(),
        "downloads" | "download" => dirs::download_dir(),
        "documents" | "docs" => dirs::document_dir(),
        "pictures" | "photos" => dirs::picture_dir(),
        "music" => dirs::audio_dir(),
        "videos" | "video" => dirs::video_dir(),
        "home" => dirs::home_dir(),
        _ => return None,
    };
    dir.map(|p| p.to_string_lossy().to_string())
}

/// Recursively collects paths under `root` whose file name contains `needle`.
fn find_files(root: &std::path::Path, needle: &str, hits: &mut Vec<String>, depth: usize) {
    if depth > 6 || hits.len() >= 200 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.contains(needle) {
            hits.push(path.to_string_lossy().to_string());
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            find_files(&path, needle, hits, depth + 1);
        }
    }
}