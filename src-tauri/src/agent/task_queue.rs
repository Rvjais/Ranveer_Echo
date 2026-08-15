use super::error_handler::CancelFlag;
use super::executor::AgentExecutor;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    High = 1,
    Normal = 2,
    Low = 3,
}

#[derive(Clone)]
pub struct Task {
    pub task_id: String,
    pub goal: String,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub error: String,
    /// Set when the user cancels; the executor checks it between steps.
    pub cancel: Arc<CancelFlag>,
}

struct QueueState {
    queue: Vec<Task>,
    tasks: HashMap<String, Task>,
}

pub struct TaskQueue {
    state: Mutex<QueueState>,
    notify: Notify,
}

impl TaskQueue {
    pub fn new() -> Self {
        TaskQueue {
            state: Mutex::new(QueueState {
                queue: Vec::new(),
                tasks: HashMap::new(),
            }),
            notify: Notify::new(),
        }
    }

    pub fn submit(&self, goal: &str) -> String {
        let task_id = Uuid::new_v4().to_string()[..8].to_string();
        let task = Task {
            task_id: task_id.clone(),
            goal: goal.to_string(),
            status: TaskStatus::Pending,
            result: None,
            error: String::new(),
            cancel: Arc::new(CancelFlag::new()),
        };
        {
            let mut state = self.state.lock().unwrap();
            state.queue.push(task.clone());
            state.tasks.insert(task_id.clone(), task);
        }
        self.notify.notify_one();
        println!(
            "[TaskQueue] Task queued: [{task_id}] {}",
            goal.chars().take(60).collect::<String>()
        );
        task_id
    }

    pub fn cancel(&self, task_id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(task) = state.tasks.get_mut(task_id) else {
            return false;
        };
        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            return false;
        }
        task.status = TaskStatus::Cancelled;
        // Signal the running executor (if any) so it stops between steps.
        task.cancel.cancel();
        println!("[TaskQueue] Task cancelled: [{task_id}]");
        true
    }

    pub fn get_status(&self, task_id: &str) -> Option<Value> {
        let state = self.state.lock().unwrap();
        let task = state.tasks.get(task_id)?;
        Some(json!({
            "task_id": task.task_id,
            "goal": task.goal,
            "status": task.status.as_str(),
            "result": task.result,
            "error": task.error,
        }))
    }

    pub fn get_all_statuses(&self) -> Vec<Value> {
        let state = self.state.lock().unwrap();
        state
            .tasks
            .values()
            .map(|t| {
                json!({
                    "task_id": t.task_id,
                    "goal": t.goal.chars().take(50).collect::<String>(),
                    "status": t.status.as_str(),
                })
            })
            .collect()
    }

    fn next_task(state: &mut QueueState) -> Option<Task> {
        for i in 0..state.queue.len() {
            if state.queue[i].status == TaskStatus::Pending {
                let mut task = state.queue.remove(i);
                task.status = TaskStatus::Running;
                // Mirror the Running status into the canonical map entry so
                // agent_task_status / agent_tasks show it while executing.
                if let Some(stored) = state.tasks.get_mut(&task.task_id) {
                    stored.status = TaskStatus::Running;
                }
                return Some(task);
            }
        }
        None
    }

    pub async fn worker_loop(&self) {
        loop {
            let task = {
                let mut state = self.state.lock().unwrap();
                Self::next_task(&mut state)
            };
            let Some(mut task) = task else {
                self.notify.notified().await;
                continue;
            };
            // Cancelled while still queued (before the worker picked it up).
            if task.cancel.is_set() {
                println!("[TaskQueue] Skipping cancelled task: [{}]", task.task_id);
                if let Some(stored) = self.state.lock().unwrap().tasks.get_mut(&task.task_id) {
                    stored.status = TaskStatus::Cancelled;
                }
                continue;
            }

            println!(
                "[TaskQueue] Running: [{task_id}] {goal}",
                task_id = task.task_id,
                goal = task.goal.chars().take(60).collect::<String>()
            );
            let (result, success) = AgentExecutor::new()
                .execute(&task.goal, Some(&task.cancel))
                .await;
            let task_id = task.task_id.clone();
            {
                let mut state = self.state.lock().unwrap();
                if task.cancel.is_set() {
                    task.status = TaskStatus::Cancelled;
                    task.result = None;
                } else if success {
                    task.status = TaskStatus::Completed;
                    task.result = Some(result.clone());
                } else {
                    task.status = TaskStatus::Failed;
                    task.error = result.clone();
                    task.result = Some(result.clone());
                }
                if let Some(stored) = state.tasks.get_mut(&task.task_id) {
                    *stored = task.clone();
                }
            }
            println!("[TaskQueue] Finished: [{task_id}] status={}", task.status.as_str());
        }
    }
}

static QUEUE: Lazy<TaskQueue> = Lazy::new(TaskQueue::new);
static QUEUE_STARTED: AtomicBool = AtomicBool::new(false);

pub fn get_queue() -> &'static TaskQueue {
    if !QUEUE_STARTED.swap(true, Ordering::SeqCst) {
        std::thread::Builder::new()
            .name("task-queue-worker".to_string())
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build task queue runtime");
                let queue: &'static TaskQueue = &QUEUE;
                // A panic in a task must not kill the queue permanently: catch
                // it and restart the worker loop.
                loop {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        rt.block_on(async {
                            queue.worker_loop().await;
                        });
                    }));
                    if result.is_ok() {
                        break;
                    }
                    println!("[TaskQueue] Worker panicked — restarting the worker loop");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            })
            .expect("failed to spawn task queue worker thread");
        println!("[TaskQueue] Worker started");
    }
    &QUEUE
}