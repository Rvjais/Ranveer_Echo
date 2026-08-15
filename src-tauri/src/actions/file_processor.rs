use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn _detect_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "svg" | "ico" => "image",
        "mp4" | "avi" | "mov" | "mkv" | "wmv" | "flv" | "webm" | "m4v" | "3gp" => "video",
        "mp3" | "wav" | "ogg" | "m4a" | "aac" | "flac" | "wma" | "opus" => "audio",
        "py" | "js" | "ts" | "jsx" | "tsx" | "html" | "css" | "java" | "c" | "cpp" | "cs"
        | "go" | "rs" | "rb" | "php" | "swift" | "kt" | "sh" | "bash" | "ps1" | "lua" | "r"
        | "m" | "sql" | "yaml" => "code",
        "zip" | "rar" | "tar" | "gz" | "7z" | "bz2" | "xz" => "archive",
        "pdf" => "pdf",
        "docx" | "doc" => "docx",
        "txt" | "md" | "rst" | "log" => "text",
        "csv" | "tsv" => "csv",
        "xlsx" | "xls" | "ods" => "excel",
        "json" => "json",
        "xml" => "xml",
        "pptx" | "ppt" => "pptx",
        _ => "unknown",
    }
}

/// A lean native port of the Python file_processor. Handles text/code/json
/// operations without heavy dependencies; image describe is local-only.
pub fn file_processor(args: Value) -> String {
    let path_str = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("info").to_lowercase();
    if path_str.is_empty() {
        return "No file path provided for file_processor.".to_string();
    }
    let path = Path::new(path_str);
    if !path.exists() {
        return format!("File not found: {path_str}");
    }
    let kind = _detect_type(path);

    match (kind, action.as_str()) {
        (_, "info") => {
            let meta = std::fs::metadata(path).map(|m| {
                let size = m.len();
                let size_str = if size < 1024 {
                    format!("{size} B")
                } else if size < 1024 * 1024 {
                    format!("{:.1} KB", size as f64 / 1024.0)
                } else {
                    format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
                };
                (size_str, m)
            });
            match meta {
                Ok((size_str, _)) => {
                    format!("{} — type: {kind}, size: {size_str}", path.display())
                }
                Err(e) => format!("Could not read file info: {e}"),
            }
        }
        ("text" | "code" | "json" | "xml" | "csv", "read" | "extract_text") => {
            std::fs::read_to_string(path)
                .map(|s| {
                    let preview: String = s.chars().take(2000).collect();
                    format!("```\n{}\n```", preview)
                })
                .unwrap_or_else(|e| format!("Read error: {e}"))
        }
        ("text" | "code" | "json" | "xml" | "csv", "word_count" | "wordcount" | "stats") => {
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    let words = s.split_whitespace().count();
                    let lines = s.lines().count();
                    let chars = s.chars().count();
                    format!("Word count: {words}\nLines: {lines}\nCharacters: {chars}")
                }
                Err(e) => format!("Read error: {e}"),
            }
        }
        ("json", "validate") => {
            match std::fs::read_to_string(path) {
                Ok(s) => match serde_json::from_str::<Value>(&s) {
                    Ok(v) => format!("Valid JSON. Top-level keys: {}", v.as_object().map(|o| o.keys().count()).unwrap_or(0)),
                    Err(e) => format!("Invalid JSON: {e}"),
                },
                Err(e) => format!("Read error: {e}"),
            }
        }
        ("json", "format") => {
            match std::fs::read_to_string(path) {
                Ok(s) => match serde_json::from_str::<Value>(&s) {
                    Ok(v) => {
                        let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
                        let out = format!("{}.pretty.json", path.with_extension("").display());
                        let _ = std::fs::write(&out, pretty);
                        format!("Formatted JSON written to {out}")
                    }
                    Err(e) => format!("Invalid JSON: {e}"),
                },
                Err(e) => format!("Read error: {e}"),
            }
        }
        ("image", "describe" | "analyze" | "read" | "extract_text" | "ocr") => {
            describe_image(path)
        }
        (_, "describe" | "analyze" | "summarize" | "explain") => {
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    let preview: String = s.chars().take(3000).collect();
                    format!(
                        "Analyzed {} (type: {kind}) via local preview.\nContent preview:\n{}",
                        path.display(),
                        preview
                    )
                }
                Err(e) => format!("Could not read file for analysis: {e}"),
            }
        }
        ("archive", "list" | "extract") => {
            let r = Command::new("tar").args(["-tf", path_str]).output();
            match r {
                Ok(o) if o.status.success() => {
                    let listing = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    let lines: Vec<&str> = listing.lines().take(40).collect();
                    format!("Archive contents:\n{}", lines.join("\n"))
                }
                _ => {
                    let r2 = Command::new("tar").args(["-xf", path_str]).output();
                    match r2 {
                        Ok(_) => format!("Extracted {path_str} to the current directory."),
                        Err(e) => format!("Archive handling error: {e}"),
                    }
                }
            }
        }
        _ => {
            format!(
                "file_processor: {path_str} is type '{kind}'. Supported here: info, read, word_count/stats, validate/format (json), image info/describe (local), archive list/extract."
            )
        }
    }
}

/// Describes an image. No cloud AI: only reports local file metadata, since a
/// vision-capable local model isn't installed.
fn describe_image(path: &Path) -> String {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return format!("Could not read image: {e}"),
    };
    let size_kb = meta.len() as f64 / 1024.0;
    let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    };
    format!(
        "Image found ({mime}, {:.1} KB). Visual description requires a vision-capable local model (e.g. `ollama pull llama3.2-vision`); none is installed, so I can't describe the picture's contents locally.",
        size_kb
    )
}