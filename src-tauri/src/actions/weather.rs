use serde_json::Value;

pub fn weather_action(parameters: Value) -> String {
    let city = parameters
        .get("city")
        .or_else(|| parameters.get("location"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if city.is_empty() {
        return "Sir, the city is missing for the weather report.".to_string();
    }

    let time = parameters
        .get("time")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "today".to_string());

    let search_query = format!("weather in {city} {time}");
    let url = format!("https://www.google.com/search?q={}", urlencode(&search_query));

    let opened = open_browser(&url);
    if !opened {
        return "Sir, I couldn't open the browser for the weather report.".to_string();
    }

    format!("Showing the weather for {city}, {time}, sir.")
}

/// Percent-encodes a query string for use in a URL (UTF-8 safe).
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

fn open_browser(url: &str) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .is_ok()
    }
}