use crate::config;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

static DB_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub struct WorkspaceStore {
    db_path: PathBuf,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn clean_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn title_from_message(message: &str) -> String {
    let text = clean_text(message);
    let text = text.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    let stripped: String = text
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '&' || *c == '/' || *c == '-')
        .collect();
    let words: Vec<&str> = stripped.split_whitespace().collect();
    if words.is_empty() {
        return "New Conversation".to_string();
    }
    let stop = [
        "a", "an", "the", "and", "or", "to", "for", "of", "in", "on", "with", "my", "your",
        "please",
    ];
    let mut pieces: Vec<String> = Vec::new();
    for (idx, word) in words.iter().take(7).enumerate() {
        let low = word.to_lowercase();
        if idx > 0 && stop.contains(&low.as_str()) {
            pieces.push(low);
        } else {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                let rest: String = chars.collect();
                pieces.push(format!("{}{}", first.to_uppercase(), rest.to_lowercase()));
            }
        }
    }
    let title = pieces.join(" ");
    title.chars().take(64).collect()
}

fn serialize_attachments(attachments: &Option<Value>) -> String {
    match attachments {
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()),
        None => "[]".to_string(),
    }
}

fn connect(path: &PathBuf) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    // Required for `ON DELETE CASCADE` to actually fire; SQLite defaults it OFF
    // per-connection, which was orphaning a conversation's messages on delete.
    conn.pragma_update(None, "foreign_keys", true).ok();
    Ok(conn)
}

impl WorkspaceStore {
    pub fn new() -> Self {
        let db_path = config::workspace_store_path();
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        WorkspaceStore { db_path }
    }

    fn init(&self) {
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            let _ = conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS conversations (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    pinned INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS messages (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    attachments_json TEXT NOT NULL DEFAULT '[]',
                    FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
                );
                CREATE TABLE IF NOT EXISTS memories (
                    id TEXT PRIMARY KEY,
                    content TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL,
                    source_conversation TEXT
                );
                CREATE TABLE IF NOT EXISTS state (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                "#,
            );
        }
    }

    fn get_state(&self, key: &str) -> Option<String> {
        let _guard = DB_LOCK.lock().unwrap();
        let conn = connect(&self.db_path).ok()?;
        conn.query_row(
            "SELECT value FROM state WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
    }

    fn set_state(&self, key: &str, value: &str) {
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            let _ = conn.execute(
                "INSERT INTO state(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            );
        }
    }

    pub fn get_active_conversation_id(&self) -> Option<String> {
        self.get_state("active_conversation_id")
    }

    pub fn set_active_conversation_id(&self, id: &str) {
        self.set_state("active_conversation_id", id);
    }

    pub fn create_conversation(&self, title: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let stamp = now_ms();
        let title = if title.trim().is_empty() {
            "New Conversation".to_string()
        } else {
            title.to_string()
        };
        {
            let _guard = DB_LOCK.lock().unwrap();
            if let Ok(conn) = connect(&self.db_path) {
                let _ = conn.execute(
                    "INSERT INTO conversations(id, title, created_at, updated_at, pinned) VALUES(?1, ?2, ?3, ?4, 0)",
                    params![id, title, stamp, stamp],
                );
            }
        }
        self.set_active_conversation_id(&id);
        id
    }

    pub fn get_conversation(&self, id: &str) -> Option<Value> {
        let _guard = DB_LOCK.lock().unwrap();
        let conn = connect(&self.db_path).ok()?;
        let mut stmt = conn
            .prepare("SELECT id, title, created_at, updated_at, pinned FROM conversations WHERE id = ?1")
            .ok()?;
        let convo = stmt
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT role, content, timestamp, attachments_json FROM messages WHERE conversation_id = ?1 ORDER BY timestamp ASC, rowid ASC",
            )
            .ok()?;
        let rows = stmt
            .query_map(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .ok()?;
        let mut messages = Vec::new();
        for r in rows.flatten() {
            let attachments = serde_json::from_str::<Value>(&r.3).unwrap_or(json!([]));
            messages.push(json!({
                "role": r.0,
                "content": r.1,
                "timestamp": r.2,
                "attachments": attachments,
            }));
        }
        Some(json!({
            "id": convo.0,
            "title": convo.1,
            "createdAt": convo.2,
            "updatedAt": convo.3,
            "pinned": convo.4 != 0,
            "messages": messages,
        }))
    }

    pub fn conversation_has_messages(&self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        let _guard = DB_LOCK.lock().unwrap();
        let Ok(conn) = connect(&self.db_path) else {
            return false;
        };
        conn.query_row(
            "SELECT 1 FROM messages WHERE conversation_id = ?1 LIMIT 1",
            params![id],
            |_| Ok(()),
        )
        .is_ok()
    }

    pub fn rollover_active_conversation_on_startup(&self) -> String {
        if let Some(current) = self.get_active_conversation_id() {
            if !self.conversation_has_messages(&current) {
                return current;
            }
        }
        self.create_conversation("New Conversation")
    }

    pub fn ensure_active_conversation(&self, first_message: Option<&str>) -> String {
        if let Some(id) = self.get_active_conversation_id() {
            if self.get_conversation(&id).is_some() {
                return id;
            }
        }
        let title = title_from_message(first_message.unwrap_or("New Conversation"));
        self.create_conversation(&title)
    }

    pub fn list_conversations(&self, search: &str) -> Vec<Value> {
        let search = clean_text(search);
        let _guard = DB_LOCK.lock().unwrap();
        let Ok(conn) = connect(&self.db_path) else {
            return vec![];
        };
        let query = if search.is_empty() {
            "SELECT id, title, created_at, updated_at, pinned FROM conversations ORDER BY pinned DESC, updated_at DESC, created_at DESC".to_string()
        } else {
            "SELECT id, title, created_at, updated_at, pinned FROM conversations WHERE title LIKE ?1 ESCAPE '\\' OR id IN (SELECT conversation_id FROM messages WHERE content LIKE ?1 ESCAPE '\\') ORDER BY pinned DESC, updated_at DESC, created_at DESC".to_string()
        };
        let mut results = Vec::new();
        let mut stmt = match conn.prepare(&query) {
            Ok(s) => s,
            Err(_) => return results,
        };
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<(String, String, i64, i64, i64)> {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        };
        let search_like = format!("%{}%", Self::escape_like(&search));
        let rows = if search.is_empty() {
            stmt.query_map([], map_row)
        } else {
            stmt.query_map([&search_like], map_row)
        };
        if let Ok(rows) = rows {
            for r in rows.flatten() {
                results.push(json!({
                    "id": r.0,
                    "title": r.1,
                    "createdAt": r.2,
                    "updatedAt": r.3,
                    "pinned": r.4 != 0,
                }));
            }
        }
        results
    }

    /// Escapes SQL LIKE wildcards so user search text matches literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub fn grouped_conversations(&self, search: &str) -> Value {
        let items = self.list_conversations(search);
        let now = now_ms();
        let day_ms = 86_400_000_i64;
        let mut groups: Vec<(&str, Vec<Value>)> = Vec::new();
        for (label, delta_bound) in [
            ("Today", 1),
            ("Yesterday", 2),
            ("Previous 7 Days", 8),
        ] {
            let bucket: Vec<Value> = items
                .iter()
                .filter(|item| {
                    let delta = (now - item["updatedAt"].as_i64().unwrap_or(0)) as f64;
                    let bound = delta_bound as f64;
                    delta >= 0.0 && delta < bound * day_ms as f64
                })
                .cloned()
                .collect();
            if !bucket.is_empty() {
                groups.push((label, bucket));
            }
        }
        let older: Vec<Value> = items
            .iter()
            .filter(|item| {
                let delta = now - item["updatedAt"].as_i64().unwrap_or(0);
                delta >= day_ms * 8
            })
            .cloned()
            .collect();
        if !older.is_empty() {
            groups.push(("Older", older));
        }
        let mut obj = serde_json::Map::new();
        for (label, bucket) in groups {
            obj.insert(label.to_string(), json!(bucket));
        }
        Value::Object(obj)
    }

    pub fn rename_conversation(&self, id: &str, title: &str) {
        let title = clean_text(title);
        let title = if title.is_empty() {
            "Conversation".to_string()
        } else {
            title.chars().take(80).collect()
        };
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            let _ = conn.execute(
                "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
                params![title, now_ms(), id],
            );
        }
    }

    pub fn pin_conversation(&self, id: &str, pinned: bool) {
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            let _ = conn.execute(
                "UPDATE conversations SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
                params![if pinned { 1 } else { 0 }, now_ms(), id],
            );
        }
    }

    pub fn delete_conversation(&self, id: &str) {
        let is_active = self.get_active_conversation_id().as_deref() == Some(id);
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            let _ = conn.execute("DELETE FROM conversations WHERE id = ?1", params![id]);
            if is_active {
                let _ = conn.execute("DELETE FROM state WHERE key = 'active_conversation_id'", []);
            }
        }
    }

    pub fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        timestamp: Option<i64>,
        attachments: Option<Value>,
    ) {
        let content = clean_text(content);
        if content.is_empty() {
            return;
        }
        let stamp = timestamp.unwrap_or_else(now_ms);
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            let convo_title: Option<String> = conn
                .query_row(
                    "SELECT title FROM conversations WHERE id = ?1",
                    params![conversation_id],
                    |row| row.get(0),
                )
                .ok();
            if convo_title.is_none() {
                // conversation doesn't exist; release lock then create then insert
                drop(conn);
                drop(_guard);
                let new_title = if role == "user" {
                    title_from_message(&content)
                } else {
                    "New Conversation".to_string()
                };
                let new_id = self.create_conversation(&new_title);
                return self.append_message(
                    &new_id,
                    role,
                    &content,
                    Some(stamp),
                    attachments,
                );
            }
            let mid = Uuid::new_v4().to_string();
            let _ = conn.execute(
                "INSERT INTO messages(id, conversation_id, role, content, timestamp, attachments_json) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    mid,
                    conversation_id,
                    role,
                    content,
                    stamp,
                    serialize_attachments(&attachments)
                ],
            );
            if role == "user" {
                let current_title: String = conn
                    .query_row(
                        "SELECT title FROM conversations WHERE id = ?1",
                        params![conversation_id],
                        |row| row.get(0),
                    )
                    .unwrap_or_default();
                if matches!(
                    current_title.as_str(),
                    "New Conversation" | "Conversation" | ""
                ) {
                    let _ = conn.execute(
                        "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
                        params![title_from_message(&content), stamp, conversation_id],
                    );
                } else {
                    let _ = conn.execute(
                        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                        params![now_ms(), conversation_id],
                    );
                }
            } else {
                let _ = conn.execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![now_ms(), conversation_id],
                );
            }
        }
    }

    pub fn record_chat(
        &self,
        role: &str,
        content: &str,
        conversation_id: Option<&str>,
        attachments: Option<Value>,
    ) -> String {
        let conversation_id = if role == "user" {
            conversation_id
                .map(|c| c.to_string())
                .unwrap_or_else(|| self.ensure_active_conversation(Some(content)))
        } else {
            let cid = conversation_id.map(|c| c.to_string());
            cid.unwrap_or_else(|| {
                self.get_active_conversation_id()
                    .unwrap_or_else(|| self.ensure_active_conversation(None))
            })
        };
        self.append_message(&conversation_id, role, content, None, attachments);
        if role == "user" {
            self.ingest_memory(content, &conversation_id);
        }
        self.set_active_conversation_id(&conversation_id);
        conversation_id
    }

    fn ingest_memory(&self, user_text: &str, conversation_id: &str) {
        let text = clean_text(user_text);
        if text.len() < 4 {
            return;
        }
        let text_low = text.to_lowercase();
        let mut candidates: Vec<String> = Vec::new();
        let patterns = [
            r"i(?:'m| am)?\s+(?:a|an)?\s*(?:software|python|web|desktop|mobile|full[- ]?stack|frontend|backend|data|ai|ml)?\s*(?:developer|engineer|creator|builder|user)?\s*(?:who\s+)?(?:use|uses|using|like|likes|love|loves|prefer|prefers|own|owns|work with|work on)\s+(.+)",
            r"my favorite\s+(.+)",
            r"i (?:use|prefer|like|love|own|build|built|am using|work with|work on)\s+(.+)",
            r"i'm\s+(.+)",
        ];
        for pattern in patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(cap) = re.captures(&text_low) {
                    if let Some(m) = cap.get(1) {
                        let value = clean_text(m.as_str());
                        if !value.is_empty() {
                            candidates.push(value.chars().take(220).collect());
                        }
                    }
                }
            }
        }
        if candidates.is_empty() {
            let triggers = [
                "prefers",
                "uses",
                "likes",
                "owns",
                "working on",
            ];
            if triggers.iter().any(|t| text_low.contains(t)) {
                candidates.push(text.chars().take(220).collect());
            }
        }
        let _guard = DB_LOCK.lock().unwrap();
        if let Ok(conn) = connect(&self.db_path) {
            for item in candidates {
                let normalized = clean_text(&item);
                if normalized.is_empty() {
                    continue;
                }
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO memories(id, content, created_at, source_conversation) VALUES(?1, ?2, ?3, ?4)",
                    params![Uuid::new_v4().to_string(), normalized, now_ms(), conversation_id],
                );
            }
        }
    }

    pub fn search_memories(&self, query: &str, limit: usize) -> Vec<Value> {
        let query = clean_text(query);
        if query.is_empty() {
            return vec![];
        }
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|t| t.to_lowercase())
            .filter(|t| t.len() > 2)
            .take(8)
            .collect();
        let _guard = DB_LOCK.lock().unwrap();
        let Ok(conn) = connect(&self.db_path) else {
            return vec![];
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, content, created_at, source_conversation FROM memories ORDER BY created_at DESC",
        ) else {
            return vec![];
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        }) else {
            return vec![];
        };

        let mut scored: Vec<(i32, Value)> = Vec::new();
        for r in rows.flatten() {
            let content_low = r.1.to_lowercase();
            let mut score: i32 = 0;
            for token in &tokens {
                if content_low.contains(token.as_str()) {
                    score += 3;
                }
            }
            let word_re = Regex::new(r"[a-z0-9]+").unwrap();
            let words: std::collections::HashSet<String> = word_re
                .find_iter(&content_low)
                .map(|m| m.as_str().to_string())
                .collect();
            let token_set: std::collections::HashSet<String> = tokens.iter().cloned().collect();
            let overlap = token_set.intersection(&words).count() as i32;
            score += overlap;
            if score > 0 {
                scored.push((
                    score,
                    json!({
                        "id": r.0,
                        "content": r.1,
                        "createdAt": r.2,
                        "sourceConversation": r.3,
                    }),
                ));
            }
        }
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1["createdAt"].as_i64().unwrap_or(0).cmp(&a.1["createdAt"].as_i64().unwrap_or(0)))
        });
        scored.into_iter().take(limit).map(|(_, v)| v).collect()
    }

    pub fn memory_context(&self, query: &str, limit: usize) -> String {
        let memories = self.search_memories(query, limit);
        if memories.is_empty() {
            return String::new();
        }
        let mut lines = vec!["Relevant Memories:".to_string()];
        for m in memories {
            lines.push(format!("- {}", m["content"].as_str().unwrap_or("")));
        }
        lines.join("\n")
    }

    pub fn get_message_payloads(&self, conversation_id: &str) -> Vec<Value> {
        self.get_conversation(conversation_id)
            .map(|c| c["messages"].as_array().cloned().unwrap_or_default())
            .unwrap_or_default()
    }

    pub fn export_conversation(&self, conversation_id: &str, path: &str) -> Result<String, String> {
        let convo = self
            .get_conversation(conversation_id)
            .ok_or_else(|| "Conversation not found".to_string())?;
        let target = std::path::Path::new(path);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(&convo).map_err(|e| e.to_string())?;
        std::fs::write(target, text).map_err(|e| e.to_string())?;
        Ok(target.to_string_lossy().to_string())
    }

    pub fn all_memories(&self) -> Vec<Value> {
        let _guard = DB_LOCK.lock().unwrap();
        let Ok(conn) = connect(&self.db_path) else {
            return vec![];
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, content, created_at, source_conversation FROM memories ORDER BY created_at DESC",
        ) else {
            return vec![];
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        }) else {
            return vec![];
        };
        rows.flatten()
            .map(|r| {
                json!({
                    "id": r.0,
                    "content": r.1,
                    "createdAt": r.2,
                    "sourceConversation": r.3,
                })
            })
            .collect()
    }
}

pub fn store() -> &'static WorkspaceStore {
    static STORE: Lazy<WorkspaceStore> = Lazy::new(|| {
        let s = WorkspaceStore::new();
        s.init();
        s
    });
    &STORE
}