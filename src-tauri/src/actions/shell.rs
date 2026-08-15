use serde_json::Value;
use std::process::Command;

/// Commands that are destructive or system-wide. They require explicit
/// `confirmed: "yes"` (the model must first ask the user).
const DANGEROUS: [&str; 14] = [
    "remove-item -recurse",
    "remove-item -force",
    "rmdir /s",
    "rd /s",
    "del /s",
    "del /f",
    "erase /s",
    "rm -rf",
    "shutdown",
    "restart-computer",
    "stop-computer",
    "format",
    "diskpart",
    "reg delete",
];

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

/// Runs a shell command (Windows: powershell/cmd). Used for computer_control
/// style commands where a direct system call is needed. Destructive commands
/// require the user to confirm first (confirmed: "yes").
pub fn shell_command(parameters: Value) -> String {
    let command = parameters
        .get("command")
        .or_else(|| parameters.get("task"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if command.is_empty() {
        return "No command provided, sir.".to_string();
    }

    let low = command.to_lowercase();
    if DANGEROUS.iter().any(|d| low.contains(d)) {
        if !is_confirmed(&parameters) {
            return "That command can modify or damage your system. If you really want it, tell the user and call again with confirmed=yes.".to_string();
        }
        println!("[shell] Running user-confirmed dangerous command: {command}");
    }

    #[cfg(windows)]
    {
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if out.status.success() {
                    if !stdout.is_empty() {
                        format!("Done, sir. {stdout}")
                    } else if !stderr.is_empty() {
                        format!("Done with output: {stderr}")
                    } else {
                        "Done, sir.".to_string()
                    }
                } else {
                    format!("The command returned an error, sir: {}", stderr)
                }
            }
            Err(e) => format!("Failed to run the command, sir: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("sh").arg("-c").arg(&command).output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if out.status.success() {
                    format!("Done, sir. {stdout}")
                } else {
                    format!("The command returned an error, sir: {stdout}")
                }
            }
            Err(e) => format!("Failed to run the command, sir: {e}"),
        }
    }
}