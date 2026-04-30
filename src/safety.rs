use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::ToolPermissionMode;
use crate::notifications::NotificationDispatcher;
use crate::storage;

// ---------------------------------------------------------------------------
// Action classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionTier {
    AutoAllowed,
    RequiresPermission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Low,
    Normal,
    High,
}

// ---------------------------------------------------------------------------
// Permission request / result / decision
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub action: String,
    pub description: String,
    pub rationale: String,
    pub urgency: Urgency,
    pub wait: bool,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionResult {
    Approved { message: Option<String> },
    Denied { reason: Option<String> },
    Queued { request_id: String },
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub request_id: String,
    pub approved: bool,
    pub decided_at: DateTime<Utc>,
    pub decided_via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionRequirement {
    pub reason: String,
    pub fingerprint: String,
}

// ---------------------------------------------------------------------------
// Action log / transcript
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLog {
    pub action_type: String,
    pub description: String,
    pub tier: ActionTier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptStatus {
    Complete,
    Interrupted,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbientTranscript {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub status: TranscriptStatus,
    pub provider: String,
    pub model: String,
    pub actions: Vec<ActionLog>,
    pub pending_permissions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub compactions: u32,
    pub memories_modified: u32,
    /// Full conversation transcript (markdown) for email notifications
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
}

// ---------------------------------------------------------------------------
// Tier-1 (auto-allowed) action names
// ---------------------------------------------------------------------------

const AUTO_ALLOWED: &[&str] = &[
    "read",
    "glob",
    "grep",
    "ls",
    "memory",
    "todo",
    "todowrite",
    "todoread",
    "conversation_search",
    "session_search",
    "codesearch",
];

const ASK_MODE_REQUIRES: &[&str] = &[
    "bash",
    "write",
    "edit",
    "multiedit",
    "patch",
    "apply_patch",
    "browser",
    "gmail",
    "mcp",
    "open",
    "webfetch",
    "websearch",
    "codesearch",
    "subagent",
    "swarm",
    "schedule",
    "selfdev",
    "debug_socket",
];

pub fn tool_permission_requirement(
    mode: ToolPermissionMode,
    tool_name: &str,
    input: &Value,
    working_dir: Option<&Path>,
) -> Option<ToolPermissionRequirement> {
    let name = canonical_tool_name(tool_name);
    let reason = match mode {
        ToolPermissionMode::Ask => ask_mode_reason(&name, input, working_dir),
        ToolPermissionMode::Autopilot => None,
    }?;

    Some(ToolPermissionRequirement {
        reason,
        fingerprint: tool_permission_fingerprint(&name, input),
    })
}

pub fn tool_permission_fingerprint(tool_name: &str, input: &Value) -> String {
    format!("{}:{}", canonical_tool_name(tool_name), input)
}

fn canonical_tool_name(tool_name: &str) -> String {
    let lower = tool_name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "communicate" => "swarm".to_string(),
        "task" | "task_runner" => "subagent".to_string(),
        "launch" => "open".to_string(),
        "shell_exec" => "bash".to_string(),
        "file_read" => "read".to_string(),
        "file_write" => "write".to_string(),
        "file_edit" => "edit".to_string(),
        "file_glob" => "glob".to_string(),
        "file_grep" => "grep".to_string(),
        "todoread" | "todowrite" | "todo_read" | "todo_write" => "todo".to_string(),
        other if other.starts_with("mcp__") => "mcp".to_string(),
        other => other.to_string(),
    }
}

fn ask_mode_reason(name: &str, input: &Value, working_dir: Option<&Path>) -> Option<String> {
    if name == "batch" || name == "invalid" || name == "todo" || name == "bg" {
        return None;
    }
    if name == "memory" {
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(action.as_str(), "remember" | "forget" | "tag" | "link") {
            return Some("memory mutation requires approval in ask mode".to_string());
        }
        return None;
    }
    if name == "read" && input_references_sensitive_path(input, working_dir) {
        return Some("reading a sensitive file requires approval in ask mode".to_string());
    }
    if matches!(name, "grep" | "glob" | "agentgrep")
        && input_references_sensitive_path(input, working_dir)
    {
        return Some("searching a sensitive path requires approval in ask mode".to_string());
    }
    if ASK_MODE_REQUIRES.iter().any(|tool| tool == &name) {
        return Some(format!("tool '{}' requires approval in ask mode", name));
    }
    None
}

fn input_references_sensitive_path(input: &Value, working_dir: Option<&Path>) -> bool {
    let keys = ["file_path", "path", "target", "glob"];
    keys.iter().any(|key| {
        input
            .get(*key)
            .and_then(Value::as_str)
            .map(|value| path_looks_sensitive(value, working_dir))
            .unwrap_or(false)
    })
}

fn path_looks_sensitive(path: &str, working_dir: Option<&Path>) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    let expanded = expand_path(path, working_dir);
    let lower = expanded.to_string_lossy().to_ascii_lowercase();
    let file_name = expanded
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();

    if lower.contains("/.jcode/auth")
        || lower.contains("/.jcode/servers.json")
        || lower.contains("/.claude.json")
        || lower.contains("/.claude/")
        || lower.contains("/.ssh/")
        || lower.contains("/.aws/")
        || lower.contains("/.kube/")
    {
        return true;
    }

    file_name == ".env"
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.contains("secret")
        || file_name.contains("token")
        || file_name.contains("credential")
        || file_name == "id_rsa"
        || file_name == "id_ed25519"
}

fn expand_path(path: &str, working_dir: Option<&Path>) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    let path_buf = PathBuf::from(path);
    if path_buf.is_absolute() {
        path_buf
    } else if let Some(working_dir) = working_dir {
        working_dir.join(path_buf)
    } else {
        path_buf
    }
}

fn permission_fingerprint(context: Option<&Value>) -> Option<&str> {
    context?
        .get("tool_permission")?
        .get("fingerprint")?
        .as_str()
}

// ---------------------------------------------------------------------------
// SafetySystem
// ---------------------------------------------------------------------------

pub struct SafetySystem {
    queue: Mutex<Vec<PermissionRequest>>,
    history: Mutex<Vec<Decision>>,
    actions: Mutex<Vec<ActionLog>>,
    notifier: NotificationDispatcher,
}

impl SafetySystem {
    /// Create a new SafetySystem, loading persisted queue/history from disk.
    pub fn new() -> Self {
        let queue: Vec<PermissionRequest> = queue_path()
            .ok()
            .and_then(|p| storage::read_json(&p).ok())
            .unwrap_or_default();

        let history: Vec<Decision> = history_path()
            .ok()
            .and_then(|p| storage::read_json(&p).ok())
            .unwrap_or_default();

        SafetySystem {
            queue: Mutex::new(queue),
            history: Mutex::new(history),
            actions: Mutex::new(Vec::new()),
            notifier: NotificationDispatcher::new(),
        }
    }

    /// Classify an action name into a tier.
    pub fn classify(&self, action: &str) -> ActionTier {
        let lower = action.to_lowercase();
        if AUTO_ALLOWED.iter().any(|&a| a == lower) {
            ActionTier::AutoAllowed
        } else {
            ActionTier::RequiresPermission
        }
    }

    pub fn pending_tool_permission_request(&self, fingerprint: &str) -> Option<PermissionRequest> {
        self.queue.lock().ok().and_then(|q| {
            q.iter()
                .find(|request| {
                    permission_fingerprint(request.context.as_ref()) == Some(fingerprint)
                })
                .cloned()
        })
    }

    pub fn has_recent_tool_permission_approval(&self, fingerprint: &str) -> bool {
        const APPROVAL_TTL_SECS: i64 = 10 * 60;
        let cutoff = Utc::now() - chrono::Duration::seconds(APPROVAL_TTL_SECS);
        self.history.lock().is_ok_and(|history| {
            history.iter().rev().any(|decision| {
                decision.approved
                    && decision.decided_at >= cutoff
                    && permission_fingerprint(decision.context.as_ref()) == Some(fingerprint)
            })
        })
    }

    /// Submit a permission request. Returns `Queued` with the request id.
    pub fn request_permission(&self, request: PermissionRequest) -> PermissionResult {
        let request_id = request.id.clone();
        let action = request.action.clone();
        let description = request.description.clone();
        if let Ok(mut q) = self.queue.lock() {
            q.push(request);
            let _ = persist_queue(&q);
        }
        // Send high-priority notification for permission request
        self.notifier
            .dispatch_permission_request(&action, &description, &request_id);
        PermissionResult::Queued { request_id }
    }

    /// Expire pending permission requests that can no longer be serviced
    /// because their originating session is no longer active.
    pub fn expire_dead_session_requests(&self, via: &str) -> Result<Vec<String>> {
        let mut expired: Vec<(String, String)> = Vec::new();

        if let Ok(mut q) = self.queue.lock() {
            let mut retained: Vec<PermissionRequest> = Vec::with_capacity(q.len());
            for req in q.drain(..) {
                if let Some(reason) = stale_request_reason(&req) {
                    expired.push((req.id.clone(), reason));
                } else {
                    retained.push(req);
                }
            }
            *q = retained;
            let _ = persist_queue(&q);
        }

        if expired.is_empty() {
            return Ok(Vec::new());
        }

        if let Ok(mut h) = self.history.lock() {
            for (request_id, reason) in &expired {
                h.push(Decision {
                    request_id: request_id.clone(),
                    approved: false,
                    decided_at: Utc::now(),
                    decided_via: via.to_string(),
                    message: Some(format!(
                        "Expired automatically: {}. Original agent is no longer active.",
                        reason
                    )),
                    action: None,
                    context: None,
                });
            }
            let _ = persist_history(&h);
        }

        Ok(expired.into_iter().map(|(id, _)| id).collect())
    }

    /// Record a user decision (approve / deny) for a pending request.
    pub fn record_decision(
        &self,
        request_id: &str,
        approved: bool,
        via: &str,
        message: Option<String>,
    ) -> Result<()> {
        let matched_request = self
            .queue
            .lock()
            .ok()
            .and_then(|q| q.iter().find(|r| r.id == request_id).cloned());

        // Remove from queue
        if let Ok(mut q) = self.queue.lock() {
            q.retain(|r| r.id != request_id);
            let _ = persist_queue(&q);
        }

        let decision = Decision {
            request_id: request_id.to_string(),
            approved,
            decided_at: Utc::now(),
            decided_via: via.to_string(),
            message,
            action: matched_request
                .as_ref()
                .map(|request| request.action.clone()),
            context: matched_request.and_then(|request| request.context),
        };

        if let Ok(mut h) = self.history.lock() {
            h.push(decision);
            let _ = persist_history(&h);
        }

        Ok(())
    }

    /// Return all pending permission requests.
    pub fn pending_requests(&self) -> Vec<PermissionRequest> {
        self.queue.lock().map(|q| q.clone()).unwrap_or_default()
    }

    /// Append an action to the in-memory log.
    pub fn log_action(&self, log: ActionLog) {
        if let Ok(mut actions) = self.actions.lock() {
            actions.push(log);
        }
    }

    /// Generate a human-readable summary of logged actions.
    pub fn generate_summary(&self) -> String {
        let actions = self.actions.lock().map(|a| a.clone()).unwrap_or_default();
        let pending = self.pending_requests();

        let mut lines: Vec<String> = Vec::new();

        if actions.is_empty() && pending.is_empty() {
            return "No actions recorded.".to_string();
        }

        // Separate auto vs permission-required
        let auto: Vec<&ActionLog> = actions
            .iter()
            .filter(|a| a.tier == ActionTier::AutoAllowed)
            .collect();
        let perm: Vec<&ActionLog> = actions
            .iter()
            .filter(|a| a.tier == ActionTier::RequiresPermission)
            .collect();

        if !auto.is_empty() {
            lines.push("Done (auto-allowed):".to_string());
            for a in &auto {
                lines.push(format!("- {} — {}", a.action_type, a.description));
            }
        }

        if !perm.is_empty() {
            lines.push(String::new());
            lines.push("Done (with permission):".to_string());
            for a in &perm {
                lines.push(format!("- {} — {}", a.action_type, a.description));
            }
        }

        if !pending.is_empty() {
            lines.push(String::new());
            lines.push("Needs your review:".to_string());
            for r in &pending {
                lines.push(format!(
                    "- [{:?}] {} — {}",
                    r.urgency, r.action, r.description
                ));
            }
        }

        lines.join("\n")
    }

    /// Persist a transcript to ~/.jcode/ambient/transcripts/{timestamp}.json
    pub fn save_transcript(&self, transcript: &AmbientTranscript) -> Result<()> {
        let dir = storage::jcode_dir()?.join("ambient").join("transcripts");
        storage::ensure_dir(&dir)?;

        let filename = transcript.started_at.format("%Y-%m-%d-%H%M%S").to_string();
        let path = dir.join(format!("{}.json", filename));
        storage::write_json(&path, transcript)
    }
}

impl Default for SafetySystem {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

fn queue_path() -> Result<std::path::PathBuf> {
    Ok(storage::jcode_dir()?.join("safety").join("queue.json"))
}

fn history_path() -> Result<std::path::PathBuf> {
    Ok(storage::jcode_dir()?.join("safety").join("history.json"))
}

fn persist_queue(queue: &[PermissionRequest]) -> Result<()> {
    let path = queue_path()?;
    storage::write_json(&path, queue)
}

fn persist_history(history: &[Decision]) -> Result<()> {
    let path = history_path()?;
    storage::write_json(&path, history)
}

// ---------------------------------------------------------------------------
// File-based permission decision (for IMAP poller / external callers)
// ---------------------------------------------------------------------------

/// Record a permission decision by directly manipulating the queue/history JSON files.
/// Used by the IMAP reply poller which doesn't have access to the live SafetySystem instance.
pub fn record_permission_via_file(
    request_id: &str,
    approved: bool,
    via: &str,
    message: Option<String>,
) -> Result<()> {
    let qp = queue_path()?;
    if let Some(parent) = qp.parent() {
        storage::ensure_dir(parent)?;
    }
    let mut queue: Vec<PermissionRequest> = if qp.exists() {
        storage::read_json(&qp).unwrap_or_default()
    } else {
        Vec::new()
    };
    let matched_request = queue.iter().find(|r| r.id == request_id).cloned();
    queue.retain(|r| r.id != request_id);
    persist_queue(&queue)?;

    let hp = history_path()?;
    if let Some(parent) = hp.parent() {
        storage::ensure_dir(parent)?;
    }
    let mut history: Vec<Decision> = if hp.exists() {
        storage::read_json(&hp).unwrap_or_default()
    } else {
        Vec::new()
    };
    history.push(Decision {
        request_id: request_id.to_string(),
        approved,
        decided_at: Utc::now(),
        decided_via: via.to_string(),
        message,
        action: matched_request
            .as_ref()
            .map(|request| request.action.clone()),
        context: matched_request.and_then(|request| request.context),
    });
    persist_history(&history)?;

    Ok(())
}

/// Expire stale permission requests directly via queue/history files.
/// Used by processes that don't hold the live SafetySystem instance.
pub fn expire_stale_permissions_via_file(via: &str) -> Result<Vec<String>> {
    let qp = queue_path()?;
    if let Some(parent) = qp.parent() {
        storage::ensure_dir(parent)?;
    }
    let mut queue: Vec<PermissionRequest> = if qp.exists() {
        storage::read_json(&qp).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut expired: Vec<(String, String)> = Vec::new();
    queue.retain(|req| {
        if let Some(reason) = stale_request_reason(req) {
            expired.push((req.id.clone(), reason));
            false
        } else {
            true
        }
    });
    persist_queue(&queue)?;

    if expired.is_empty() {
        return Ok(Vec::new());
    }

    let hp = history_path()?;
    if let Some(parent) = hp.parent() {
        storage::ensure_dir(parent)?;
    }
    let mut history: Vec<Decision> = if hp.exists() {
        storage::read_json(&hp).unwrap_or_default()
    } else {
        Vec::new()
    };
    for (request_id, reason) in &expired {
        history.push(Decision {
            request_id: request_id.clone(),
            approved: false,
            decided_at: Utc::now(),
            decided_via: via.to_string(),
            message: Some(format!(
                "Expired automatically: {}. Original agent is no longer active.",
                reason
            )),
            action: None,
            context: None,
        });
    }
    persist_history(&history)?;

    Ok(expired.into_iter().map(|(id, _)| id).collect())
}

fn stale_request_reason(request: &PermissionRequest) -> Option<String> {
    let session_id = request_session_id(request)?;
    let mut session = match crate::session::Session::load(&session_id) {
        Ok(s) => s,
        Err(_) => return Some(format!("owner session '{}' was not found", session_id)),
    };

    // Refresh crash status based on PID if needed.
    if session.detect_crash() {
        let _ = session.save();
    }

    if session.status == crate::session::SessionStatus::Active {
        None
    } else {
        Some(format!(
            "owner session '{}' is {}",
            session_id,
            session.status.display()
        ))
    }
}

fn request_session_id(request: &PermissionRequest) -> Option<String> {
    let context = request.context.as_ref()?;

    context
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            context
                .get("requester")
                .and_then(|r| r.get("session_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

// ---------------------------------------------------------------------------
// ID generation helper
// ---------------------------------------------------------------------------

/// Generate a unique permission request id: `req_{timestamp}_{random}`
pub fn new_request_id() -> String {
    crate::id::new_id("req")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_home<F, T>(f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let _guard = crate::storage::lock_test_env();
        let prev_home = std::env::var_os("JCODE_HOME");
        let temp = tempfile::TempDir::new().expect("create temp dir");
        crate::env::set_var("JCODE_HOME", temp.path());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        match prev_home {
            Some(value) => crate::env::set_var("JCODE_HOME", value),
            None => crate::env::remove_var("JCODE_HOME"),
        }

        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }

    #[test]
    fn test_classify_auto_allowed() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            assert_eq!(sys.classify("read"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("glob"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("grep"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("ls"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("memory"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("todo"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("todowrite"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("todoread"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("conversation_search"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("session_search"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("codesearch"), ActionTier::AutoAllowed);
        });
    }

    #[test]
    fn test_classify_requires_permission() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            assert_eq!(sys.classify("bash"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("write"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("edit"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("multiedit"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("patch"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("apply_patch"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("communicate"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("open"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("launch"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("webfetch"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("websearch"), ActionTier::RequiresPermission);
            assert_eq!(sys.classify("unknown_tool"), ActionTier::RequiresPermission);
        });
    }

    #[test]
    fn test_classify_case_insensitive() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            assert_eq!(sys.classify("Read"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("GLOB"), ActionTier::AutoAllowed);
            assert_eq!(sys.classify("Bash"), ActionTier::RequiresPermission);
        });
    }

    #[test]
    fn test_request_permission_returns_queued() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            let baseline = sys.pending_requests().len();
            let req = PermissionRequest {
                id: "req_test_1".to_string(),
                action: "create_pull_request".to_string(),
                description: "Create PR for test fixes".to_string(),
                rationale: "Found failing tests".to_string(),
                urgency: Urgency::Normal,
                wait: false,
                created_at: Utc::now(),
                context: None,
            };

            let result = sys.request_permission(req);
            match result {
                PermissionResult::Queued { request_id } => {
                    assert_eq!(request_id, "req_test_1");
                }
                _ => panic!("Expected Queued result"),
            }

            assert_eq!(sys.pending_requests().len(), baseline + 1);
        });
    }

    #[test]
    fn test_record_decision_removes_from_queue() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            let baseline = sys.pending_requests().len();
            let req = PermissionRequest {
                id: "req_test_2".to_string(),
                action: "push".to_string(),
                description: "Push to origin".to_string(),
                rationale: "Ready for review".to_string(),
                urgency: Urgency::Low,
                wait: false,
                created_at: Utc::now(),
                context: None,
            };

            sys.request_permission(req);
            assert_eq!(sys.pending_requests().len(), baseline + 1);

            sys.record_decision("req_test_2", true, "tui", Some("looks good".to_string()))
                .unwrap();
            assert_eq!(sys.pending_requests().len(), baseline);
        });
    }

    #[test]
    fn test_log_action_and_summary() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            sys.log_action(ActionLog {
                action_type: "memory_consolidation".to_string(),
                description: "Merged 2 duplicate memories".to_string(),
                tier: ActionTier::AutoAllowed,
                details: None,
                timestamp: Utc::now(),
            });
            sys.log_action(ActionLog {
                action_type: "edit".to_string(),
                description: "Fixed typo in README".to_string(),
                tier: ActionTier::RequiresPermission,
                details: None,
                timestamp: Utc::now(),
            });

            let summary = sys.generate_summary();
            assert!(summary.contains("memory_consolidation"));
            assert!(summary.contains("edit"));
            assert!(summary.contains("Done (auto-allowed)"));
            assert!(summary.contains("Done (with permission)"));
        });
    }

    #[test]
    fn test_empty_summary() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            let summary = sys.generate_summary();
            assert_eq!(summary, "No actions recorded.");
        });
    }

    #[test]
    fn test_new_request_id_format() {
        with_temp_home(|| {
            let id = new_request_id();
            assert!(id.starts_with("req_"));
        });
    }

    #[test]
    fn test_record_permission_via_file() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            let baseline = sys.pending_requests().len();
            let req = PermissionRequest {
                id: "req_file_test".to_string(),
                action: "push".to_string(),
                description: "Push to origin".to_string(),
                rationale: "Ready for review".to_string(),
                urgency: Urgency::Low,
                wait: false,
                created_at: Utc::now(),
                context: None,
            };
            sys.request_permission(req);
            assert_eq!(sys.pending_requests().len(), baseline + 1);

            record_permission_via_file("req_file_test", true, "email_reply", None).unwrap();

            let sys2 = SafetySystem::new();
            let still_pending = sys2
                .pending_requests()
                .iter()
                .any(|r| r.id == "req_file_test");
            assert!(
                !still_pending,
                "request should have been removed from queue"
            );
        });
    }

    #[test]
    fn test_tool_permission_ask_mode_requires_shell_and_sensitive_read() {
        with_temp_home(|| {
            let shell = tool_permission_requirement(
                ToolPermissionMode::Ask,
                "bash",
                &serde_json::json!({"command": "echo ok"}),
                None,
            );
            assert!(shell.is_some());

            let normal_read = tool_permission_requirement(
                ToolPermissionMode::Ask,
                "read",
                &serde_json::json!({"file_path": "src/main.rs"}),
                Some(Path::new("/tmp/project")),
            );
            assert!(normal_read.is_none());

            let sensitive_read = tool_permission_requirement(
                ToolPermissionMode::Ask,
                "read",
                &serde_json::json!({"file_path": "~/.claude.json"}),
                None,
            );
            assert!(sensitive_read.is_some());
        });
    }

    #[test]
    fn test_tool_permission_autopilot_allows_all_tool_calls() {
        with_temp_home(|| {
            let cases = [
                (
                    "bash",
                    serde_json::json!({"command": "rm -rf /tmp/example"}),
                ),
                (
                    "gmail",
                    serde_json::json!({"action": "send", "to": "x@example.com"}),
                ),
                (
                    "mcp",
                    serde_json::json!({"action": "connect", "server": "test"}),
                ),
                (
                    "browser",
                    serde_json::json!({"action": "click", "selector": "button"}),
                ),
                ("read", serde_json::json!({"file_path": "~/.claude.json"})),
            ];

            for (tool, input) in cases {
                let requirement =
                    tool_permission_requirement(ToolPermissionMode::Autopilot, tool, &input, None);
                assert!(
                    requirement.is_none(),
                    "{tool} should not require permission in autopilot mode"
                );
            }
        });
    }

    #[test]
    fn test_tool_permission_approval_is_matched_by_fingerprint() {
        with_temp_home(|| {
            let sys = SafetySystem::new();
            let input = serde_json::json!({"command": "echo ok"});
            let fingerprint = tool_permission_fingerprint("bash", &input);
            let req = PermissionRequest {
                id: "req_tool_test".to_string(),
                action: "tool:bash".to_string(),
                description: "Run tool 'bash'".to_string(),
                rationale: "test".to_string(),
                urgency: Urgency::Normal,
                wait: true,
                created_at: Utc::now(),
                context: Some(serde_json::json!({
                    "tool_permission": {
                        "fingerprint": fingerprint,
                    }
                })),
            };
            sys.request_permission(req);
            assert!(sys.pending_tool_permission_request(&fingerprint).is_some());

            sys.record_decision("req_tool_test", true, "test", None)
                .unwrap();

            assert!(sys.pending_tool_permission_request(&fingerprint).is_none());
            assert!(sys.has_recent_tool_permission_approval(&fingerprint));
        });
    }
}
