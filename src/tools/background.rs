use std::collections::{HashMap, VecDeque};
use std::fmt::Write;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for BackgroundTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackgroundTaskStatus::Running => write!(f, "Running"),
            BackgroundTaskStatus::Completed => write!(f, "Completed"),
            BackgroundTaskStatus::Failed => write!(f, "Failed"),
            BackgroundTaskStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

pub struct BackgroundTask {
    pub label: String,
    pub tool_name: String,
    pub arguments: String,
    pub status: BackgroundTaskStatus,
    pub output_chunks: Vec<String>,
    pub result: Option<String>,
    pub success: Option<bool>,
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    pub abort_tx: Option<oneshot::Sender<()>>,
    pub acknowledged: bool,
}

#[derive(Debug, Clone)]
pub struct CompletedTaskResult {
    pub label: String,
    pub tool_name: String,
    pub result: String,
    pub success: bool,
    pub elapsed: Duration,
}

pub struct BackgroundTaskManager {
    tasks: HashMap<String, BackgroundTask>,
    completed_queue: VecDeque<CompletedTaskResult>,
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

impl BackgroundTaskManager {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            completed_queue: VecDeque::new(),
        }
    }

    pub fn active_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|t| t.status == BackgroundTaskStatus::Running)
            .count()
    }

    pub fn has_running_tasks(&self) -> bool {
        self.tasks
            .values()
            .any(|t| t.status == BackgroundTaskStatus::Running)
    }

    /// Insert a task, rejecting if a task with the same label is already
    /// Running. Completed/failed/cancelled tasks with the same label get
    /// overwritten.
    pub fn insert(&mut self, task: BackgroundTask) -> Result<(), String> {
        if let Some(existing) = self.tasks.get(&task.label) {
            if existing.status == BackgroundTaskStatus::Running {
                return Err(format!(
                    "task with label '{}' is already running",
                    task.label
                ));
            }
        }
        self.tasks.insert(task.label.clone(), task);
        Ok(())
    }

    pub fn get_mut(&mut self, label: &str) -> Option<&mut BackgroundTask> {
        self.tasks.get_mut(label)
    }

    pub fn append_output(&mut self, label: &str, chunk: String) {
        if let Some(task) = self.tasks.get_mut(label) {
            task.output_chunks.push(chunk);
        }
    }

    /// Mark a task as completed or failed, set its result and timing, clear
    /// the abort channel, and push a `CompletedTaskResult` into the queue.
    pub fn complete(&mut self, label: &str, result: String, success: bool) {
        if let Some(task) = self.tasks.get_mut(label) {
            task.status = if success {
                BackgroundTaskStatus::Completed
            } else {
                BackgroundTaskStatus::Failed
            };
            task.result = Some(result.clone());
            task.success = Some(success);
            task.finished_at = Some(Instant::now());
            task.abort_tx = None;

            let elapsed = task
                .finished_at
                .unwrap()
                .duration_since(task.started_at);

            self.completed_queue.push_back(CompletedTaskResult {
                label: task.label.clone(),
                tool_name: task.tool_name.clone(),
                result,
                success,
                elapsed,
            });
        }
    }

    /// Send the abort signal and mark the task as cancelled. Returns an
    /// error if the task does not exist or is not currently running.
    pub fn cancel(&mut self, label: &str) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(label)
            .ok_or_else(|| format!("task '{}' not found", label))?;

        if task.status != BackgroundTaskStatus::Running {
            return Err(format!("task '{}' is not running", label));
        }

        if let Some(tx) = task.abort_tx.take() {
            let _ = tx.send(());
        }

        task.status = BackgroundTaskStatus::Cancelled;
        task.finished_at = Some(Instant::now());
        Ok(())
    }

    /// Return a formatted status string for a single task. If the task is
    /// no longer running, mark it as acknowledged.
    pub fn status_one(&mut self, label: &str) -> Result<String, String> {
        let task = self
            .tasks
            .get_mut(label)
            .ok_or_else(|| format!("task '{}' not found", label))?;

        let elapsed = task
            .finished_at
            .unwrap_or_else(Instant::now)
            .duration_since(task.started_at);

        let mut out = String::new();
        let _ = writeln!(out, "Label: {}", task.label);
        let _ = writeln!(out, "Tool: {}", task.tool_name);
        let _ = writeln!(out, "Status: {}", task.status);
        let _ = writeln!(out, "Elapsed: {}", format_duration(elapsed));

        let chunks = &task.output_chunks;
        let start = chunks.len().saturating_sub(100);
        if !chunks[start..].is_empty() {
            let _ = writeln!(out, "Output (last {} chunks):", chunks[start..].len());
            for chunk in &chunks[start..] {
                let _ = writeln!(out, "  {}", chunk);
            }
        }

        if task.status != BackgroundTaskStatus::Running {
            task.acknowledged = true;
        }

        Ok(out)
    }

    /// One-line-per-task summary of ALL tasks.
    pub fn summary(&self) -> String {
        if self.tasks.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        for task in self.tasks.values() {
            let elapsed = task
                .finished_at
                .unwrap_or_else(Instant::now)
                .duration_since(task.started_at);
            let _ = writeln!(
                out,
                "[{}] {} — {} ({})",
                task.label,
                task.tool_name,
                task.status,
                format_duration(elapsed)
            );
        }
        out
    }

    /// Like `summary` but only for running tasks.
    pub fn running_summary(&self) -> String {
        let mut out = String::new();
        for task in self.tasks.values() {
            if task.status != BackgroundTaskStatus::Running {
                continue;
            }
            let elapsed = Instant::now().duration_since(task.started_at);
            let _ = writeln!(
                out,
                "[{}] {} — Running ({})",
                task.label,
                task.tool_name,
                format_duration(elapsed)
            );
        }
        out
    }

    /// Drain the completed-task queue into a `Vec`.
    pub fn drain_completed(&mut self) -> Vec<CompletedTaskResult> {
        self.completed_queue.drain(..).collect()
    }

    /// Remove tasks that are not running AND have been acknowledged.
    pub fn clear_acknowledged(&mut self) {
        self.tasks.retain(|_, t| {
            t.status == BackgroundTaskStatus::Running || !t.acknowledged
        });
    }

    /// Abort all running tasks and clear everything.
    pub fn clear_all(&mut self) {
        for task in self.tasks.values_mut() {
            if let Some(tx) = task.abort_tx.take() {
                let _ = tx.send(());
            }
        }
        self.tasks.clear();
        self.completed_queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a minimal running task with the given label.
    fn make_task(label: &str) -> BackgroundTask {
        BackgroundTask {
            label: label.to_string(),
            tool_name: "shell".to_string(),
            arguments: "{}".to_string(),
            status: BackgroundTaskStatus::Running,
            output_chunks: Vec::new(),
            result: None,
            success: None,
            started_at: Instant::now(),
            finished_at: None,
            abort_tx: None,
            acknowledged: false,
        }
    }

    /// Helper to build a running task that includes an abort channel.
    fn make_task_with_abort(label: &str) -> (BackgroundTask, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        let task = BackgroundTask {
            label: label.to_string(),
            tool_name: "shell".to_string(),
            arguments: "{}".to_string(),
            status: BackgroundTaskStatus::Running,
            output_chunks: Vec::new(),
            result: None,
            success: None,
            started_at: Instant::now(),
            finished_at: None,
            abort_tx: Some(tx),
            acknowledged: false,
        };
        (task, rx)
    }

    #[test]
    fn new_manager_is_empty() {
        let mut mgr = BackgroundTaskManager::new();
        assert_eq!(mgr.active_count(), 0);
        assert!(mgr.summary().is_empty());
        assert!(mgr.drain_completed().is_empty());
    }

    #[test]
    fn insert_and_active_count() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("build")).unwrap();
        assert_eq!(mgr.active_count(), 1);
    }

    #[test]
    fn insert_duplicate_running_label_rejected() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("build")).unwrap();
        let result = mgr.insert(make_task("build"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already running"));
    }

    #[test]
    fn complete_moves_to_queue() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("build")).unwrap();
        mgr.complete("build", "done".to_string(), true);

        assert_eq!(mgr.active_count(), 0);

        let drained = mgr.drain_completed();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].label, "build");
        assert_eq!(drained[0].result, "done");
        assert!(drained[0].success);
    }

    #[test]
    fn cancel_running_task() {
        let mut mgr = BackgroundTaskManager::new();
        let (task, _rx) = make_task_with_abort("build");
        mgr.insert(task).unwrap();

        mgr.cancel("build").unwrap();

        let t = mgr.get_mut("build").unwrap();
        assert_eq!(t.status, BackgroundTaskStatus::Cancelled);
        assert!(t.finished_at.is_some());
    }

    #[test]
    fn cancel_nonexistent_task_errors() {
        let mut mgr = BackgroundTaskManager::new();
        let result = mgr.cancel("nope");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn append_output_and_status() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("build")).unwrap();

        mgr.append_output("build", "line 1".to_string());
        mgr.append_output("build", "line 2".to_string());

        let status = mgr.status_one("build").unwrap();
        assert!(status.contains("line 1"));
        assert!(status.contains("line 2"));
    }

    #[test]
    fn status_one_acknowledges_finished_task() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("build")).unwrap();
        mgr.complete("build", "ok".to_string(), true);

        // Not yet acknowledged before status_one is called.
        assert!(!mgr.tasks.get("build").unwrap().acknowledged);

        let _status = mgr.status_one("build").unwrap();
        assert!(mgr.tasks.get("build").unwrap().acknowledged);
    }

    #[test]
    fn clear_acknowledged_retains_running() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("running-task")).unwrap();
        mgr.insert(make_task("done-task")).unwrap();

        // Complete and acknowledge the second task.
        mgr.complete("done-task", "finished".to_string(), true);
        let _status = mgr.status_one("done-task").unwrap();

        mgr.clear_acknowledged();

        assert!(mgr.get_mut("running-task").is_some());
        assert!(mgr.get_mut("done-task").is_none());
    }

    #[test]
    fn insert_reuses_label_of_finished_task() {
        let mut mgr = BackgroundTaskManager::new();
        mgr.insert(make_task("build")).unwrap();
        mgr.complete("build", "v1".to_string(), true);

        // Re-inserting with the same label should succeed because the
        // original is no longer running.
        let result = mgr.insert(make_task("build"));
        assert!(result.is_ok());
        assert_eq!(mgr.active_count(), 1);
    }
}
