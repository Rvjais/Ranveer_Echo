use serde_json::{json, Value};

pub struct SearchResult {
    pub title: String,
    pub snippet: String,
    pub url: String,
}

/// DuckDuckGo HTML search without external deps. Parses real result anchors
/// (`class="result__a"`) and snippets (`class="result__snippet"`), sending a
/// browser User-Agent so DDG doesn't 403 the request.
fn ddg_search(query: &str, max_results: usize) -> Vec<SearchResult> {
    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencode(query));
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let Ok(resp) = client.get(&url).send() else {
        return vec![];
    };
    if !resp.status().is_success() {
        return vec![];
    }
    let Ok(text) = resp.text() else {
        return vec![];
    };

    // Extract result anchors in document order.
    let mut anchors: Vec<(String, String)> = Vec::new();
    let mut pos = 0usize;
    while anchors.len() < max_results {
        let Some(rel) = text[pos..].find("class=\"result__a\"") else {
            break;
        };
        let anchor_start = pos + rel;
        let after = &text[anchor_start..];
        let Some(href_rel) = after.find("href=") else {
            break;
        };
        let Some(href) = extract_href(&after[href_rel..]) else {
            pos = anchor_start + 1;
            continue;
        };
        let Some(tag_end_rel) = after[href_rel..].find('>') else {
            break;
        };
        let tag_end = href_rel + tag_end_rel;
        let title_body = &after[tag_end + 1..];
        let Some(close_rel) = title_body.find("</a>") else {
            break;
        };
        let title = strip_tags(&title_body[..close_rel]);
        anchors.push((href, title));
        pos = anchor_start + tag_end + close_rel + 4;
    }

    // Extract snippets (they are plain text inside their own <a> tags).
    let mut snippets: Vec<String> = Vec::new();
    let mut pos = 0usize;
    while snippets.len() < max_results {
        let Some(rel) = text[pos..].find("class=\"result__snippet\"") else {
            break;
        };
        let after = &text[pos + rel..];
        let Some(body_rel) = after.find('>') else {
            break;
        };
        let body = &after[body_rel + 1..];
        let Some(close_rel) = body.find("</a>") else {
            break;
        };
        let snippet = strip_tags(&body[..close_rel]);
        if !snippet.is_empty() {
            snippets.push(snippet);
        }
        pos = pos + rel + body_rel + close_rel + 4;
    }

    let mut results: Vec<SearchResult> = Vec::new();
    for (i, (url, title)) in anchors.into_iter().enumerate() {
        results.push(SearchResult {
            title,
            snippet: snippets.get(i).cloned().unwrap_or_default(),
            url,
        });
    }
    results
}

fn extract_href(raw: &str) -> Option<String> {
    let idx = raw.find('"')?;
    let rest = &raw[idx + 1..];
    let end = rest.find('"')?;
    let url = &rest[..end];
    if url.starts_with("//duckduckgo.com/l/?uddg=") {
        let encoded = url.trim_start_matches("//duckduckgo.com/l/?uddg=");
        Some(decode_url(encoded))
    } else if url.starts_with("http") {
        Some(url.to_string())
    } else if url.starts_with('/') {
        Some(format!("https://duckduckgo.com{url}"))
    } else {
        Some(url.to_string())
    }
}

fn strip_tags(raw: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut in_entity = false;
    let mut entity_buf = String::new();
    for c in raw.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if in_tag {
            continue;
        } else if c == '&' {
            in_entity = true;
            entity_buf.clear();
        } else if c == ';' && in_entity {
            in_entity = false;
            match entity_buf.as_str() {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                "nbsp" => out.push(' '),
                _ => {}
            }
        } else if in_entity {
            entity_buf.push(c);
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn decode_url(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn format_ddg(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return format!("No results found for: {query}");
    }
    let mut lines = vec![format!("Search results for: {query}\n")];
    for (i, r) in results.iter().enumerate() {
        if !r.title.is_empty() {
            lines.push(format!("{}. {}", i + 1, r.title));
        }
        if !r.snippet.is_empty() {
            lines.push(format!("   {}", r.snippet));
        }
        if !r.url.is_empty() {
            lines.push(format!("   {}", r.url));
        }
        lines.push(String::new());
    }
    lines.join("\n").trim().to_string()
}

/// Summarizes search results with the local Ollama model (no cloud).
fn ollama_search(query: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let body = json!({
        "model": crate::ollama::local_model_name(),
        "prompt": format!("Answer this search request concisely with facts:\n{query}"),
        "stream": false,
        "options": {"temperature": 0.4, "num_predict": 512}
    });
    let resp = client
        .post(format!("{}/api/generate", crate::ollama::base_url()))
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Ollama HTTP {}", resp.status()));
    }
    let data: Value = resp.json().map_err(|e| e.to_string())?;
    let text = data["response"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Err("Ollama returned an empty response.".to_string());
    }
    Ok(text)
}

fn compare(items: &[String], aspect: &str) -> String {
    let joined = items.join(", ");
    let query = format!("Compare {joined} in terms of {aspect}. Give specific facts and data.");
    if let Ok(result) = ollama_search(&query) {
        return result;
    }
    let mut all_results: Vec<(String, Vec<SearchResult>)> = Vec::new();
    for item in items {
        all_results.push((item.clone(), ddg_search(&format!("{item} {aspect}"), 3)));
    }
    let mut lines = vec![format!("Comparison — {aspect}"), "─".repeat(40).to_string()];
    for (item, results) in &all_results {
        lines.push(format!("\n▸ {item}"));
        for r in results.iter().take(2) {
            if !r.snippet.is_empty() {
                lines.push(format!("  • {}", r.snippet));
            }
        }
    }
    lines.join("\n")
}

pub fn web_search(parameters: Value) -> String {
    let query = parameters
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mode = parameters
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("search")
        .trim()
        .to_lowercase();
    let items: Vec<String> = parameters
        .get("items")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let aspect = parameters
        .get("aspect")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "general".to_string());

    if query.is_empty() && items.is_empty() {
        return "Please provide a search query, sir.".to_string();
    }

    let mode = if !items.is_empty() && mode != "compare" {
        "compare"
    } else {
        &mode
    };

    println!("[WebSearch] Query: {query:?} Mode: {mode}");

    if mode == "compare" {
        let targets: Vec<String> = if !items.is_empty() {
            items
        } else {
            vec![query]
        };
        return compare(&targets, &aspect);
    }

    let results = ddg_search(&query, 6);
    let result = format_ddg(&query, &results);
    if !results.is_empty() {
        return result;
    }
    match ollama_search(&query) {
        Ok(text) => text,
        Err(e) => format!("Search failed, sir: {e}"),
    }
}