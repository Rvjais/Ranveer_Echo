pub fn looks_like_code_request(text: &str) -> bool {
    let low = text.to_lowercase();
    [
        "build", "create", "write", "implement", "code", "python", "app", "module", "function",
        "class", "project", "script", "api", "ui", "webpage", "bot", "server", "service",
    ]
    .iter()
    .any(|word| low.contains(word))
}

pub fn looks_like_website_request(text: &str) -> bool {
    let low = text.to_lowercase();
    [
        "website",
        "web site",
        "landing page",
        "homepage",
        "home page",
        "portfolio",
        "product site",
        "business site",
        "marketing site",
        "web app",
        "frontend",
        "site",
    ]
    .iter()
    .any(|word| low.contains(word))
}

/// True only when the utterance actually contains the wake word "ranveer"
/// (as a whole word, allowing common mis-hearings). Bare "hi"/"hey" no longer
/// count — they caused the always-listening loop to fire on any greeting.
pub fn wakeword_detected(text: &str) -> bool {
    let compact: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    compact.split_whitespace().any(|w| {
        matches!(w, "ranveer" | "ranbir" | "runveer" | "ranveera" | "ranvir")
    })
}

pub fn build_task_plan(text: &str) -> Vec<String> {
    let t = text.to_lowercase();
    if ["presentation", "ppt", "slides", "deck"]
        .iter()
        .any(|w| t.contains(w))
    {
        return vec![
            "Understand the topic and goal",
            "Build a slide structure",
            "Generate and format the deck",
            "Open the finished presentation",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    if ["spreadsheet", "excel", "sheet", "table", "tracker", "budget"]
        .iter()
        .any(|w| t.contains(w))
    {
        return vec![
            "Read the data request",
            "Lay out sheets and columns",
            "Apply formulas and formatting",
            "Open the workbook",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    if ["word", "docx", "document", "report", "letter"]
        .iter()
        .any(|w| t.contains(w))
    {
        return vec![
            "Understand the document type",
            "Draft the structure and content",
            "Preserve formatting and polish",
            "Save the editable file",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    if ["website", "web site", "landing page", "saas", "dashboard", "app"]
        .iter()
        .any(|w| t.contains(w))
    {
        return vec![
            "Interpret the brief",
            "Generate frontend and backend files",
            "Launch the local preview",
            "Debug and fix launch issues if needed",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    if ["browser", "website", "google", "search", "open url", "navigate"]
        .iter()
        .any(|w| t.contains(w))
    {
        return vec![
            "Open the browser",
            "Navigate to the target page",
            "Collect the needed information",
            "Return the result",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    if ["screen", "camera", "meeting", "call", "analyze", "analyse"]
        .iter()
        .any(|w| t.contains(w))
    {
        return vec![
            "Capture the live screen or camera",
            "Inspect what is visible",
            "Answer with the important details",
            "Keep listening for follow-up commands",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    vec![
        "Understand the command",
        "Choose the right tool",
        "Execute the task",
        "Return the result",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
