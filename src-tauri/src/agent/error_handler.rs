use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorDecision {
    Retry,
    Skip,
    Replan,
    Abort,
}

pub fn analyze_error(
    step: &Value,
    error: &str,
    attempt: i32,
    max_attempts: i32,
) -> (ErrorDecision, String) {
    let err_lower = error.to_lowercase();
    let critical = step
        .get("critical")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut decision = if attempt >= max_attempts {
        ErrorDecision::Replan
    } else if err_lower.contains("unknown action")
        || err_lower.contains("not found")
        || err_lower.contains("no such")
    {
        ErrorDecision::Skip
    } else if err_lower.contains("timeout")
        || err_lower.contains("rate limit")
        || err_lower.contains("429")
        || err_lower.contains("temporarily")
        || err_lower.contains("busy")
    {
        ErrorDecision::Retry
    } else if err_lower.contains("abort")
        || err_lower.contains("permission denied")
        || err_lower.contains("access denied")
    {
        ErrorDecision::Abort
    } else {
        ErrorDecision::Replan
    };

    if decision == ErrorDecision::Skip && critical {
        decision = ErrorDecision::Replan;
    }

    let message = match decision {
        ErrorDecision::Retry => format!("That didn't work, trying again. ({error})"),
        ErrorDecision::Skip => "Skipping that step, sir.".to_string(),
        ErrorDecision::Abort => format!("Task aborted. {error}"),
        ErrorDecision::Replan => "Adjusting my approach, sir.".to_string(),
    };

    (decision, message)
}

pub fn generate_fix(
    step: &Value,
    error: &str,
    fix_suggestion: &str,
) -> Value {
    let tool = step.get("tool").and_then(|v| v.as_str()).unwrap_or("web_search");
    let desc = step
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    serde_json::json!({
        "step": step.get("step").cloned().unwrap_or(Value::Null),
        "tool": tool,
        "description": desc,
        "parameters": {
            "query": format!("{desc} ({fix_suggestion}) after error: {error}"),
        },
        "critical": step.get("critical").cloned().unwrap_or(Value::Bool(false)),
    })
}

pub struct CancelFlag {
    flag: AtomicBool,
}

impl CancelFlag {
    pub fn new() -> Self {
        CancelFlag {
            flag: AtomicBool::new(false),
        }
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}