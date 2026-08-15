use super::error_handler::{analyze_error, ErrorDecision};
use super::planner::{create_plan, replan};
use crate::actions;
use crate::or_client;
use serde_json::{json, Value};
use std::collections::HashMap;

pub type SpeakFn = Box<dyn Fn(String) + Send + Sync>;

pub struct AgentExecutor {
    pub max_replan_attempts: i32,
    pub speak: Option<SpeakFn>,
}

impl AgentExecutor {
    pub fn new() -> Self {
        AgentExecutor {
            max_replan_attempts: 2,
            speak: None,
        }
    }

    pub fn with_speaker(mut self, speak: SpeakFn) -> Self {
        self.speak = Some(speak);
        self
    }

    fn say(&self, msg: &str) {
        if let Some(f) = &self.speak {
            f(msg.to_string());
        }
    }

    pub async fn call_tool(tool: &str, parameters: &Value) -> Result<String, String> {
        match tool {
            "open_app" | "web_search" | "reminder" | "weather_report" | "desktop_control" => {
                // Some of these use a blocking HTTP client; keep them off the runtime.
                let t = tool.to_string();
                let p = parameters.clone();
                tokio::task::spawn_blocking(move || actions::execute(&t, p))
                    .await
                    .unwrap_or_else(|_| Err("Action failed to run.".to_string()))
            }
            "file_controller" => file_controller(parameters),
            "shell_command" | "cmd_control" => actions::execute("shell_command", parameters.clone()),
            // The real computer_control lives in actions::more — the old local
            // stub returned "not yet wired" for every action.
            "computer_control" => {
                Ok(crate::actions::more::computer_control(parameters.clone()))
            }
            // Everything else the planner may emit (send_message, youtube_video,
            // flight_finder, game_updater, file_processor, browser_control,
            // save_memory, ...) is routed through the shared orchestrator
            // dispatcher, so advertised tools stay dispatchable.
            _ => {
                let orch = crate::orchestrator::RanveerOrchestrator::new();
                let result = orch.execute_tool(tool, parameters).await;
                if result.starts_with("Unknown action") || result.starts_with("Unknown or unported") {
                    Err(result)
                } else {
                    Ok(result)
                }
            }
        }
    }

    pub async fn execute(
        &self,
        goal: &str,
        cancel: Option<&super::error_handler::CancelFlag>,
    ) -> (String, bool) {
        println!("\n[Executor] Goal: {goal}");

        let mut replan_attempts = 0;
        let mut completed_steps: Vec<Value> = Vec::new();
        let mut step_results: HashMap<i64, String> = HashMap::new();
        let mut plan = create_plan(goal, "").await;

        loop {
            let steps = plan
                .get("steps")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if steps.is_empty() {
                let msg = "I couldn't create a valid plan for this task, sir.";
                self.say(msg);
                return (msg.to_string(), false);
            }

            let mut success = true;
            let mut failed_step = Value::Null;
            let mut failed_error = String::new();

            for step in steps {
                if let Some(c) = cancel {
                    if c.is_set() {
                        self.say("Task cancelled, sir.");
                        return ("Task cancelled.".to_string(), false);
                    }
                }

                let step_num = step.get("step").and_then(|v| v.as_i64()).unwrap_or(-1);
                let tool = step
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("web_search")
                    .to_string();
                let desc = step
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut params = step.get("parameters").cloned().unwrap_or(json!({}));

                params = inject_context(params, &tool, &step_results, goal);

                println!("\n[Executor] Step {step_num}: [{tool}] {desc}");

                let mut attempt = 1;
                let mut step_ok = false;

                while attempt <= 3 {
                    if let Some(c) = cancel {
                        if c.is_set() {
                            break;
                        }
                    }
                    match Self::call_tool(&tool, &params).await {
                        Ok(result) => {
                            step_results.insert(step_num, result.clone());
                            completed_steps.push(step.clone());
                            println!("[Executor] Step {step_num} done: {}", result.chars().take(100).collect::<String>());
                            step_ok = true;
                            break;
                        }
                        Err(error_msg) => {
                            println!("[Executor] Step {step_num} attempt {attempt} failed: {error_msg}");
                            let (decision, user_msg) = analyze_error(&step, &error_msg, attempt, 3);
                            self.say(&user_msg);

                            match decision {
                                ErrorDecision::Retry => {
                                    attempt += 1;
                                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                    continue;
                                }
                                ErrorDecision::Skip => {
                                    println!("[Executor] Skipping step {step_num}");
                                    completed_steps.push(step.clone());
                                    step_ok = true;
                                    break;
                                }
                                ErrorDecision::Abort => {
                                    let msg = format!("Task aborted, sir. {error_msg}");
                                    self.say(&msg);
                                    return (msg, false);
                                }
                                ErrorDecision::Replan => {
                                    failed_step = step.clone();
                                    failed_error = error_msg.clone();
                                    success = false;
                                    break;
                                }
                            }
                        }
                    }
                }

                if !step_ok && failed_step.is_null() {
                    failed_step = step.clone();
                    failed_error = "Max retries exceeded".to_string();
                    success = false;
                }

                if !success {
                    break;
                }
            }

            if success {
                let summary = self.summarize(goal, &completed_steps).await;
                return (summary, true);
            }

            if replan_attempts >= self.max_replan_attempts {
                let msg = format!("Task failed after {replan_attempts} replan attempts, sir.");
                self.say(&msg);
                return (msg, false);
            }

            self.say("Adjusting my approach, sir.");
            replan_attempts += 1;
            plan = replan(goal, &completed_steps, &failed_step, &failed_error).await;
        }
    }

    async fn summarize(&self, goal: &str, completed_steps: &[Value]) -> String {
        let fallback = format!(
            "All done, sir. Completed {} steps for: {}.",
            completed_steps.len(),
            goal.chars().take(60).collect::<String>()
        );
        let steps_str: Vec<String> = completed_steps
            .iter()
            .map(|s| {
                format!(
                    "- {}",
                    s.get("description").and_then(|v| v.as_str()).unwrap_or("")
                )
            })
            .collect();
        let prompt = format!(
            "User goal: \"{goal}\"\nCompleted steps:\n{}\n\nWrite a single natural sentence summarizing what was accomplished. Address the user as 'sir'. Be direct and positive.",
            steps_str.join("\n")
        );
        match or_client::client()
            .chat(&prompt, None, None, 256, 0.5)
            .await
        {
            Ok(summary) => {
                let s = summary.trim().to_string();
                self.say(&s);
                s
            }
            Err(_) => fallback,
        }
    }
}

fn inject_context(
    mut params: Value,
    tool: &str,
    step_results: &HashMap<i64, String>,
    _goal: &str,
) -> Value {
    if step_results.is_empty() {
        return params;
    }
    if tool == "file_controller" {
        let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if action == "write" || action == "create_file" {
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if content.len() < 50 {
                let all_results: Vec<&String> = step_results
                    .values()
                    .filter(|v| v.len() > 100 && v.as_str() != "Done." && v.as_str() != "Completed.")
                    .collect();
                if !all_results.is_empty() {
                    let combined = all_results
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n---\n\n");
                    params["content"] = json!(combined);
                    println!("[Executor] Injected content from previous steps");
                }
            }
        }
    }
    params
}

fn file_controller(parameters: &Value) -> Result<String, String> {
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

    if path == "desktop" {
        if let Some(desktop) = dirs::desktop_dir() {
            path = desktop.to_string_lossy().to_string();
        }
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
        "read" => {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            Ok(text)
        }
        "list" => {
            let mut lines = Vec::new();
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                if let Ok(entry) = entry {
                    lines.push(entry.file_name().to_string_lossy().to_string());
                }
            }
            Ok(format!("Files in {path}:\n{}", lines.join("\n")))
        }
        "delete" => {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            Ok(format!("Deleted {path}"))
        }
        "disk_usage" | _ => Ok(format!("No action specified for {path}")),
    }
}