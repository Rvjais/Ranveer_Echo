use serde_json::Value;
use std::path::PathBuf;

fn desktop_path() -> PathBuf {
    if let Some(path) = dirs::desktop_dir() {
        return path;
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn desktop_control(parameters: Value) -> String {
    let action = parameters
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let task = parameters
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    println!("[desktop] action: {action} task: {}", task.chars().take(40).collect::<String>());

    match action.as_str() {
        "stats" | "list" => desktop_stats(),
        "organize" => {
            if !is_confirmed(&parameters) {
                return "Organizing moves files and shortcuts into folders on your desktop. Say you confirm and I'll call it again with confirmed=yes.".to_string();
            }
            let mode = parameters
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("by_type")
                .to_string();
            organize_desktop(&mode)
        }
        "clean" => {
            if !is_confirmed(&parameters) {
                return "Cleaning deletes temporary files (.tmp/.log) from your desktop. Say you confirm and I'll call it again with confirmed=yes.".to_string();
            }
            clean_desktop()
        }
        "current_wallpaper" => get_current_wallpaper(),
        "wallpaper" | "set_wallpaper" => {
            let path = parameters
                .get("path")
                .or_else(|| parameters.get("image"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            set_wallpaper(&path)
        }
        _ => {
            if task.is_empty() {
                "No action or task specified.".to_string()
            } else {
                format!("Desktop task '{task}' requires the assistant's AI tool loop, which is wired through the orchestrator.")
            }
        }
    }
}

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

fn desktop_stats() -> String {
    let desktop = desktop_path();
    let mut files = 0usize;
    let mut folders = 0usize;
    let mut total_size: u64 = 0;

    if let Ok(entries) = std::fs::read_dir(&desktop) {
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => folders += 1,
                Ok(ft) if ft.is_file() => {
                    files += 1;
                    total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
                _ => {}
            }
        }
    }

    let size_str = if total_size > 1024 * 1024 * 1024 {
        format!("{:.1} GB", total_size as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.1} MB", total_size as f64 / (1024.0 * 1024.0))
    };

    format!(
        "Desktop stats:\n  Files   : {files}\n  Folders : {folders}\n  Size    : {size_str}\n  Path    : {}",
        desktop.display()
    )
}

fn organize_desktop(mode: &str) -> String {
    let desktop = desktop_path();
    if !desktop.exists() {
        return "Desktop folder not found.".to_string();
    }

    let mut moved = 0usize;
    if let Ok(entries) = std::fs::read_dir(&desktop) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let folder = match mode {
                "by_date" => "By Date",
                _ => match ext.as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" => "Images",
                    "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" => "Videos",
                    "mp3" | "wav" | "flac" | "ogg" | "m4a" => "Audio",
                    "pdf" => "PDFs",
                    "doc" | "docx" | "txt" | "rtf" | "odt" => "Documents",
                    "xls" | "xlsx" | "csv" | "ods" => "Spreadsheets",
                    "ppt" | "pptx" | "odp" => "Presentations",
                    "zip" | "rar" | "7z" | "tar" | "gz" => "Archives",
                    "exe" | "msi" | "bat" | "cmd" | "ps1" => "Installers",
                    "" => "Misc",
                    _ => "Misc",
                },
            };
            let target_dir = desktop.join(&folder);
            if std::fs::create_dir_all(&target_dir).is_ok() {
                let target_path = target_dir.join(
                    entry.file_name().to_string_lossy().to_string(),
                );
                if std::fs::rename(&path, &target_path).is_ok() {
                    moved += 1;
                }
            }
        }
    }
    format!("Organized desktop by {mode}: moved {moved} file(s).")
}

fn clean_desktop() -> String {
    let desktop = desktop_path();
    let mut removed = 0usize;
    if let Ok(entries) = std::fs::read_dir(&desktop) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_file() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name == "desktop.ini" || name.ends_with(".tmp") || name.ends_with(".log") {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }
    format!("Cleaned temporary desktop files: removed {removed} file(s).")
}

#[cfg(windows)]
fn set_wallpaper(path: &str) -> String {
    if path.is_empty() {
        return "Provide the path to an image to set as wallpaper.".to_string();
    }
    if !std::path::Path::new(path).exists() {
        return format!("Image not found: {path}");
    }
    // SystemParametersInfo(SPI_SETDESKWALLPAPER=20, ..., SPIF_UPDATEINIFILE|SPIF_SENDWININICHANGE=3).
    let escaped = path.replace('\'', "''");
    let script = format!(
        "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class WP{{[DllImport(\"user32.dll\",CharSet=CharSet.Auto)]public static extern int SystemParametersInfo(int a,int u,string p,int f);}}'; [WP]::SystemParametersInfo(20,0,'{escaped}',3)"
    );
    let r = ps_command(&script);
    if r.starts_with("PowerShell error") {
        format!("Could not set the wallpaper ({r}).")
    } else {
        format!("Set the desktop wallpaper to {path}.")
    }
}

#[cfg(not(windows))]
fn set_wallpaper(_path: &str) -> String {
    "Setting the wallpaper is only supported on Windows.".to_string()
}

#[cfg(windows)]
fn ps_command(script: &str) -> String {
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
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

#[cfg(windows)]
fn get_current_wallpaper() -> String {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("reg")
        .arg("query")
        .arg("HKCU\\Control Panel\\Desktop")
        .arg("/v")
        .arg("WallPaper")
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .find(|l| l.contains("WallPaper"))
                .map(|l| l.split("REG_SZ").last().unwrap_or("").trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Could not read current wallpaper.".to_string())
        }
        Err(e) => format!("Could not read current wallpaper: {e}"),
    }
}

#[cfg(not(windows))]
fn get_current_wallpaper() -> String {
    "Wallpaper lookup is only supported on Windows.".to_string()
}