use crate::or_client;
use serde_json::{json, Value};
use std::collections::HashSet;

pub const PLANNER_PROMPT: &str = r#"You are the planning module of Ranveer, a personal AI assistant.
Your job: break any user goal into a sequence of steps using ONLY the tools listed below.

ABSOLUTE RULES:
- NEVER use generated_code or write Python scripts. It does not exist.
- NEVER reference previous step results in parameters. Every step is independent.
- Use web_search for ANY information retrieval, research, or current data.
- Use file_controller to save content to disk.
- Use cmd_control to open files or run system commands.
- Max 5 steps. Use the minimum steps needed.

AVAILABLE TOOLS AND THEIR PARAMETERS:
open_app: app_name (required)
web_search: query (required), mode ("search"/"compare"), items (list), aspect
reminder: date YYYY-MM-DD (required), time HH:MM (required), message (required)
weather_report: city (required)
desktop_control: action ("wallpaper"/"organize"/"clean"/"list"/"task"), path, task
computer_settings: action (required), description, value
send_message: receiver, message_text, platform (required), mode, media_path
youtube_video: action ("play"/"summarize"/"trending"), query
file_controller: action ("write"/"create_file"/"read"/"list"/"delete"/"move"/"copy"/"find"/"disk_usage") (required), path, name, content
cmd_control: task (required), visible
browser_control: action ("go_to"/"search"/"click"/"type"/"scroll"/"get_text"/"press"/"close") (required), url, query, text, direction
computer_control: action ("type"/"click"/"hotkey"/"press"/"scroll"/"screenshot"/"screen_find"/"screen_click") (required), text, x, y, keys, key, direction, description
screen_process: text (required), angle
flight_finder: origin, destination, date
claude_code: description (required), workspace_path

OUTPUT — return ONLY valid JSON, no markdown, no explanation, no code blocks:
{
  "goal": "...",
  "steps": [
    {
      "step": 1,
      "tool": "tool_name",
      "description": "what this step does",
      "parameters": {},
      "critical": true
    }
  ]
}"#;

pub fn looks_like_website_goal(goal: &str) -> bool {
    let text = goal.to_lowercase();
    [
        "website",
        "landing page",
        "landingpage",
        "portfolio",
        "business site",
        "product site",
        "marketing site",
        "homepage",
        "web page",
        "site for",
        "build a site",
        "create a site",
        "create a website",
    ]
    .iter()
    .any(|token| text.contains(token))
}

fn rewrite_generated_step(step: &mut Value, goal: &str) {
    if step.get("tool").and_then(|v| v.as_str()) != Some("generated_code") {
        return;
    }
    let desc = step
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or(goal)
        .to_string();
    if looks_like_website_goal(goal) {
        println!("[Planner] generated_code detected — replacing with claude_code");
        step["tool"] = json!("claude_code");
        step["parameters"] = json!({"description": desc.chars().take(1200).collect::<String>()});
    } else {
        println!("[Planner] generated_code detected — replacing with web_search");
        step["tool"] = json!("web_search");
        step["parameters"] = json!({"query": desc.chars().take(200).collect::<String>()});
    }
}

fn validate_tool_names(plan: &mut Value) {
    let allowed: HashSet<&str> = [
        "open_app",
        "web_search",
        "game_updater",
        "browser_control",
        "file_controller",
        "cmd_control",
        "claude_code",
        "screen_process",
        "send_message",
        "reminder",
        "youtube_video",
        "weather_report",
        "computer_settings",
        "desktop_control",
        "computer_control",
        "flight_finder",
        "generated_code",
    ]
    .iter()
    .copied()
    .collect();
    if let Some(steps) = plan.get_mut("steps").and_then(|v| v.as_array_mut()) {
        for step in steps.iter_mut() {
            let tool = step
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("generated_code");
            if !allowed.contains(tool)
                && step.get("tool").and_then(|v| v.as_str()) != Some("generated_code")
            {
                step["tool"] = json!("web_search");
            }
        }
    }
}

fn parse_plan_text(raw: &str) -> Result<Value, String> {
    let mut clean = raw.trim().to_string();
    clean = clean.replace("```json", "").replace("```", "").trim().trim_end_matches('`').trim().to_string();
    let plan: Value = serde_json::from_str(&clean).map_err(|e| e.to_string())?;
    if plan.get("steps").and_then(|v| v.as_array()).is_none() {
        return Err("Invalid plan structure".to_string());
    }
    Ok(plan)
}

pub async fn create_plan(goal: &str, context: &str) -> Value {
    let mut user_input = format!("Goal: {goal}");
    if !context.is_empty() {
        user_input.push_str(&format!("\n\nContext: {context}"));
    }

    let raw = or_client::client()
        .chat(
            &user_input,
            Some(PLANNER_PROMPT),
            None,
            2048,
            0.2,
        )
        .await;

    match raw {
        Ok(text) => {
            if let Ok(mut plan) = parse_plan_text(&text) {
                if let Some(steps) = plan.get_mut("steps").and_then(|v| v.as_array_mut()) {
                    for step in steps.iter_mut() {
                        rewrite_generated_step(step, goal);
                    }
                }
                validate_tool_names(&mut plan);
                let count = plan["steps"].as_array().map(|a| a.len()).unwrap_or(0);
                println!("[Planner] Plan: {count} steps");
                return plan;
            }
            println!("[Planner] JSON parse failed — falling back");
            fallback_plan(goal)
        }
        Err(e) => {
            println!("[Planner] Planning failed: {e}");
            fallback_plan(goal)
        }
    }
}

pub fn fallback_plan(goal: &str) -> Value {
    println!("[Planner] Fallback plan");
    if looks_like_website_goal(goal) {
        return json!({
            "goal": goal,
            "steps": [{
                "step": 1,
                "tool": "claude_code",
                "description": format!("Create the requested website: {goal}"),
                "parameters": {"description": goal},
                "critical": true,
            }]
        });
    }
    json!({
        "goal": goal,
        "steps": [{
            "step": 1,
            "tool": "web_search",
            "description": format!("Search for: {goal}"),
            "parameters": {"query": goal},
            "critical": true,
        }]
    })
}

pub async fn replan(
    goal: &str,
    completed_steps: &[Value],
    failed_step: &Value,
    error: &str,
) -> Value {
    let completed_summary: Vec<String> = completed_steps
        .iter()
        .map(|s| {
            format!(
                "  - Step {} ({}): DONE",
                s.get("step").unwrap_or(&json!("?")),
                s.get("tool").unwrap_or(&json!("?")).as_str().unwrap_or("?")
            )
        })
        .collect();
    let summary = if completed_summary.is_empty() {
        "  (none)".to_string()
    } else {
        completed_summary.join("\n")
    };

    let prompt = format!(
        "Goal: {goal}\n\nAlready completed:\n{summary}\n\nFailed step: [{}] {}\nError: {error}\n\nCreate a REVISED plan for the remaining work only. Do not repeat completed steps.",
        failed_step.get("tool").and_then(|v| v.as_str()).unwrap_or("?"),
        failed_step.get("description").and_then(|v| v.as_str()).unwrap_or("")
    );

    match or_client::client().chat(&prompt, Some(PLANNER_PROMPT), None, 2048, 0.2).await {
        Ok(text) => {
            let mut plan = parse_plan_text(&text).unwrap_or_else(|_| fallback_plan(goal));
            if let Some(steps) = plan.get_mut("steps").and_then(|v| v.as_array_mut()) {
                for step in steps.iter_mut() {
                    rewrite_generated_step(step, goal);
                }
            }
            validate_tool_names(&mut plan);
            println!("[Planner] Revised plan ready");
            plan
        }
        Err(e) => {
            println!("[Planner] Replan failed: {e}");
            fallback_plan(goal)
        }
    }
}