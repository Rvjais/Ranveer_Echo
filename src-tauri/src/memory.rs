use crate::config;
use chrono::Local;
use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;

static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub const MAX_VALUE_LENGTH: usize = 380;
pub const MEMORY_MAX_CHARS: usize = 2200;
const CATEGORIES: [&str; 6] = [
    "identity",
    "preferences",
    "projects",
    "relationships",
    "wishes",
    "notes",
];

pub fn empty_memory() -> Value {
    json!({
        "identity": {},
        "preferences": {},
        "projects": {},
        "relationships": {},
        "wishes": {},
        "notes": {}
    })
}

pub fn load_memory() -> Value {
    let path = config::memory_path();
    if !path.exists() {
        return empty_memory();
    }
    let _guard = LOCK.lock().unwrap();
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(mut obj)) => {
                for cat in CATEGORIES {
                    if !obj.contains_key(cat) {
                        obj.insert(cat.to_string(), Value::Object(Map::new()));
                    }
                }
                Value::Object(obj)
            }
            _ => empty_memory(),
        },
        Err(err) => {
            println!("[Memory] Load error: {err}");
            empty_memory()
        }
    }
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn truncate_value(val: &str) -> String {
    if val.len() > MAX_VALUE_LENGTH {
        let trimmed = val.trim_end();
        let bound = if trimmed.len() > MAX_VALUE_LENGTH {
            MAX_VALUE_LENGTH
        } else {
            trimmed.len()
        };
        let mut s: String = trimmed.chars().take(bound).collect();
        s.push('…');
        s
    } else {
        val.to_string()
    }
}

fn recursive_update(target: &mut Map<String, Value>, updates: &Map<String, Value>) -> bool {
    let mut changed = false;
    for (key, value) in updates {
        if value.is_null() {
            continue;
        }
        if let Some(s) = value.as_str() {
            if s.trim().is_empty() {
                continue;
            }
        }
        if value.is_object() && value.get("value").is_none() {
            if !target.contains_key(key) || !target[key].is_object() {
                target.insert(key.clone(), json!({}));
                changed = true;
            }
            if let Some(sub) = target.get_mut(key).and_then(|v| v.as_object_mut()) {
                if let Some(upd_map) = value.as_object() {
                    if recursive_update(sub, upd_map) {
                        changed = true;
                    }
                }
            }
        } else {
            let new_val = if let Some(v_map) = value.as_object() {
                if let Some(v) = v_map.get("value") {
                    truncate_value(&v.to_string().trim_matches('"'))
                } else {
                    truncate_value(&value.to_string().trim_matches('"'))
                }
            } else {
                truncate_value(&value.to_string().trim_matches('"'))
            };

            let entry = json!({"value": new_val, "updated": today()});
            let existing = target.get(key).cloned().unwrap_or(json!({}));
            let existing_val = existing
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if existing_val != new_val {
                target.insert(key.clone(), entry);
                changed = true;
            }
        }
    }
    changed
}

fn all_entries(memory: &Value) -> Vec<(String, String, Value)> {
    let mut entries = Vec::new();
    if let Some(cats) = memory.as_object() {
        for (cat, items) in cats {
            if let Some(items) = items.as_object() {
                for (key, entry) in items {
                    if entry.is_object() && entry.get("value").is_some() {
                        entries.push((cat.clone(), key.clone(), entry.clone()));
                    }
                }
            }
        }
    }
    entries
}

fn trim_to_limit(memory: &mut Value) {
    let serialized = serde_json::to_string(memory).unwrap_or_default();
    if serialized.len() <= MEMORY_MAX_CHARS {
        return;
    }
    let mut entries = all_entries(memory);
    entries.sort_by_key(|(_, _, e)| {
        e.get("updated")
            .and_then(|v| v.as_str())
            .unwrap_or("0000-00-00")
            .to_string()
    });
    for (cat, key, _) in entries {
        let sz = serde_json::to_string(memory).map(|s| s.len()).unwrap_or(0);
        if sz <= MEMORY_MAX_CHARS {
            break;
        }
        if let Some(cats) = memory.as_object_mut() {
            if let Some(items) = cats.get_mut(&cat).and_then(|v| v.as_object_mut()) {
                items.remove(&key);
                println!("[Memory] Trimmed {cat}/{key}");
            }
        }
    }
}

pub fn save_memory(memory: &mut Value) {
    if !memory.is_object() {
        return;
    }
    trim_to_limit(memory);
    let path = config::memory_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _guard = LOCK.lock().unwrap();
    if let Ok(text) = serde_json::to_string_pretty(memory) {
        // Write to a temp file then rename, so a crash mid-write can't truncate
        // long_term.json.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            // done
        } else {
            let _ = std::fs::write(&path, text);
        }
    }
}

pub fn update_memory(memory_update: &Value) -> Value {
    if !memory_update.is_object() || memory_update.as_object().map(|m| m.is_empty()).unwrap_or(true) {
        return load_memory();
    }
    let mut memory = load_memory();
    if let (Some(obj), Some(upd)) = (
        memory.as_object_mut(),
        memory_update.as_object(),
    ) {
        if recursive_update(obj, upd) {
            save_memory(&mut memory);
        }
    }
    memory
}

pub fn remember(key: &str, value: &str, category: &str) -> String {
    let cat = if CATEGORIES.contains(&category) {
        category
    } else {
        "notes"
    };
    update_memory(&json!({ cat: { key: { "value": value } } }));
    format!("Remembered: {cat}/{key} = {value}")
}

pub fn forget(key: &str, category: &str) -> String {
    let mut memory = load_memory();
    if let Some(cats) = memory.as_object_mut() {
        if let Some(items) = cats.get_mut(category).and_then(|v| v.as_object_mut()) {
            if items.remove(key).is_some() {
                save_memory(&mut memory);
                return format!("Forgotten: {category}/{key}");
            }
        }
    }
    format!("Not found: {category}/{key}")
}

pub fn format_memory_for_prompt(memory: Option<&Value>) -> String {
    let memory = match memory {
        Some(m) if m.is_object() => m,
        _ => return String::new(),
    };
    if memory.as_object().map(|m| m.is_empty()).unwrap_or(true) {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();

    let val_of = |entry: &Value| -> Option<String> {
        if entry.is_object() {
            entry.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            entry.as_str().map(|s| s.to_string())
        }
    };

    let title = |key: &str| -> String {
        let s = key.replace('_', " ");
        let mut chars = s.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
            None => key.to_string(),
        }
    };

    if let Some(identity) = memory.as_object().and_then(|m| m.get("identity")) {
        let id_fields = ["name", "age", "birthday", "city", "job", "language", "school", "nationality"];
        for field in id_fields {
            if let Some(entry) = identity.get(field) {
                if let Some(val) = val_of(entry) {
                    if !val.is_empty() {
                        lines.push(format!("{}: {val}", title(field)));
                    }
                }
            }
        }
        if let Some(items) = identity.as_object() {
            for (key, entry) in items {
                if id_fields.contains(&key.as_str()) {
                    continue;
                }
                if let Some(val) = val_of(entry) {
                    if !val.is_empty() {
                        lines.push(format!("{}: {val}", title(key)));
                    }
                }
            }
        }
    }

    let sections = [
        ("preferences", "Preferences:", 15),
        ("projects", "Active Projects / Goals:", 8),
        ("relationships", "People in their life:", 10),
        ("wishes", "Wishes / Plans / Wants:", 8),
        ("notes", "Other notes:", 8),
    ];
    for (cat, header, limit) in sections {
        if let Some(items) = memory.as_object().and_then(|m| m.get(cat)) {
            if let Some(items_obj) = items.as_object() {
                if !items_obj.is_empty() {
                    lines.push(String::new());
                    lines.push(header.to_string());
                    for (key, entry) in items_obj.iter().take(limit) {
                        if let Some(val) = val_of(entry) {
                            if !val.is_empty() {
                                lines.push(format!("  - {}: {val}", title(key)));
                            }
                        }
                    }
                }
            }
        }
    }

    if lines.is_empty() {
        return String::new();
    }

    let header = "[WHAT YOU KNOW ABOUT THIS PERSON — use naturally, never recite like a list]\n";
    let mut result = format!("{header}{}", lines.join("\n"));
    if result.len() > 2000 {
        // Truncate on a char boundary — a fixed byte offset panics on non-ASCII.
        let mut end = 1997.min(result.len());
        while end > 0 && !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
        result.push('…');
    }
    result.push('\n');
    result
}

static LAST_EXTRACT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

/// Cheap gate: only run the LLM extractor when the utterance reads like a
/// personal statement rather than a command/question, so we don't spend a
/// background model call on "open notepad".
fn worth_extracting(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.chars().count() < 6 {
        return false;
    }
    const SIGNALS: [&str; 18] = [
        "i ", "i'm", "im ", "i am", "i've", "my ", "me ", "mine", "we ", "our ", "call me",
        "name is", "i like", "i love", "i hate", "i prefer", "i work", "i live",
    ];
    SIGNALS.iter().any(|s| t.contains(s))
}

/// After a conversation turn, asks the local model to extract durable facts about
/// the user and merges them into long-term memory. Runs in the background; safe
/// to call on every turn — it self-filters, de-dupes, and drops unknown keys.
pub async fn maybe_extract(user_text: &str, assistant_reply: &str) {
    if !worth_extracting(user_text) {
        return;
    }
    {
        let mut last = LAST_EXTRACT.lock().unwrap();
        if *last == user_text {
            return;
        }
        *last = user_text.to_string();
    }

    let system = "You extract DURABLE facts about the USER worth remembering long-term: their name, age, city, job, language, preferences, ongoing projects, important people, and wishes. IGNORE transient requests, commands, and questions. Respond with ONLY a JSON object using these top-level categories: identity, preferences, projects, relationships, wishes, notes. Put each fact as \"snake_case_key\": {\"value\": \"...\"}. If there is nothing durable to store, respond with exactly {}.";
    let prompt = format!(
        "User said: \"{user_text}\"\nAssistant replied: \"{assistant_reply}\"\n\nExtract durable memory as JSON now."
    );

    let extracted = match crate::ai::chat_json(&prompt, Some(system), 300).await {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some(obj) = extracted.as_object() else {
        return;
    };
    // Keep only known categories so the model can't invent top-level keys.
    let mut filtered = Map::new();
    for cat in CATEGORIES {
        if let Some(v) = obj.get(cat) {
            if v.as_object().map(|m| !m.is_empty()).unwrap_or(false) {
                filtered.insert(cat.to_string(), v.clone());
            }
        }
    }
    if filtered.is_empty() {
        return;
    }
    let count = filtered.len();
    update_memory(&Value::Object(filtered));
    println!("[Memory] auto-extracted {count} categor(ies) from the last turn.");
}

pub fn memory_context_map() -> HashMap<String, Value> {
    let mem = load_memory();
    let mut map = HashMap::new();
    for cat in CATEGORIES {
        if let Some(v) = mem.get(cat) {
            map.insert(cat.to_string(), v.clone());
        }
    }
    map
}