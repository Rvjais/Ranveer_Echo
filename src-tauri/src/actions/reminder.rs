use chrono::{Local, NaiveDateTime};
use serde_json::Value;
use std::process::Command;

pub fn reminder(parameters: Value) -> String {
    let date_str = parameters
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let time_str = parameters
        .get("time")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let minutes = parameters
        .get("minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    let message = parameters
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Reminder")
        .to_string();

    // Relative reminders ("in N minutes") are the shape the chat model is told
    // to use; date+time is the shape the agent planner uses.
    let target_dt = if !date_str.is_empty() && !time_str.is_empty() {
        let datetime_str = format!("{date_str} {time_str}");
        match NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M") {
            Ok(dt) => dt,
            Err(_) => return "I couldn't understand that date or time format.".to_string(),
        }
    } else if minutes >= 0 {
        Local::now().naive_local() + chrono::Duration::minutes(minutes)
    } else {
        return "I need a time for this reminder: either minutes from now (e.g. minutes: 30) or a date+time (YYYY-MM-DD HH:MM).".to_string();
    };

    let now = Local::now().naive_local();
    if target_dt <= now {
        return "That time is already in the past.".to_string();
    }

    let task_name = format!("MARKReminder_{}", target_dt.format("%Y%m%d_%H%M"));
    let safe_message = message
        .replace('"', "")
        .replace('\'', "")
        .trim()
        .chars()
        .take(200)
        .collect::<String>();

    #[cfg(windows)]
    {
        let temp_dir = std::env::var("TEMP").unwrap_or_else(|_| "C:\\Temp".to_string());
        let notify_script = format!("{temp_dir}\\{task_name}.bat");
        let xml_path = format!("{temp_dir}\\{task_name}.xml");
        let vbs_path = format!("{temp_dir}\\{task_name}.vbs");
        let formatted = target_dt.format("%Y-%m-%dT%H:%M:%S").to_string();

        // VBScript popup — works from scheduled tasks and shows a real dialog,
        // unlike `msg *` which is silently dropped outside interactive sessions.
        let vbs = format!(
            "Set sh = CreateObject(\"WScript.Shell\")\nsh.Popup \"{safe_message}\", 30, \"Ranveer Reminder\", 64\n"
        );
        let _ = std::fs::write(&vbs_path, vbs);

        let batch = format!(
            "@echo off\ncscript //nologo \"{vbs_path}\"\n"
        );
        let _ = std::fs::write(&notify_script, batch);

        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>MARK Reminder: {safe_message}</Description></RegistrationInfo>
  <Triggers><TimeTrigger><StartBoundary>{formatted}</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>
  <Actions><Exec><Command>{notify_script}</Command></Exec></Actions>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <WakeToRun>true</WakeToRun>
    <ExecutionTimeLimit>PT5M</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Principals><Principal><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
</Task>"#
        );
        let _ = std::fs::write(&xml_path, xml_content);

        let result = Command::new("schtasks")
            .args([
                "/Create",
                "/TN",
                &task_name,
                "/XML",
                &xml_path,
                "/F",
            ])
            .output();

        let _ = std::fs::remove_file(&xml_path);
        let _ = std::fs::remove_file(&notify_script);
        // The .vbs is kept so the scheduled task can pop the reminder.

        match result {
            Ok(out) if out.status.success() => {
                let friendly = target_dt.format("%B %d at %I:%M %p").to_string();
                format!("Reminder set for {friendly}.")
            }
            _ => "I couldn't schedule the reminder due to a system error.".to_string(),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = &target_dt;
        "Reminders require Windows Task Scheduler (currently only supported on Windows).".to_string()
    }
}