//! Session status derivation from files.
//!
//! This module derives session status entirely from existing CLI file conventions,
//! without requiring any additional state tracking files.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::models::ReviewResult;

/// Session status derived from files in .plan-forge/<session>/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// No plan-iteration files exist yet
    Ready,
    /// Planning/review loop is in progress
    InProgress,
    /// Latest review has requires_human_input: true
    NeedsInput,
    /// Latest review passed (score >= threshold)
    Approved,
    /// Iteration count >= max without approval
    MaxTurns,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Ready => write!(f, "ready"),
            SessionStatus::InProgress => write!(f, "in_progress"),
            SessionStatus::NeedsInput => write!(f, "needs_input"),
            SessionStatus::Approved => write!(f, "approved"),
            SessionStatus::MaxTurns => write!(f, "max_turns"),
        }
    }
}

/// Information about a session derived from files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session identifier (directory name)
    pub session_id: String,
    /// Path to session directory
    pub session_dir: PathBuf,
    /// Current iteration count
    pub iteration: u32,
    /// Derived status
    pub status: SessionStatus,
    /// Latest review score (if available)
    pub latest_score: Option<f32>,
    /// Human input reason (if status is NeedsInput)
    pub input_reason: Option<String>,
    /// Plan title (if available)
    pub title: Option<String>,
}

/// Derive session status from files in a session directory.
///
/// Status derivation logic:
/// - Ready: No plan-iteration-*.json files exist
/// - NeedsInput: Latest review has requires_human_input: true
/// - Approved: Latest review passed (score >= threshold AND no requires_human_input)
/// - MaxTurns: Iteration count >= max AND not Approved AND not NeedsInput
/// - InProgress: Has plan files, none of the above
pub fn derive_status(
    session_dir: &Path,
    pass_threshold: f32,
    max_iterations: u32,
) -> anyhow::Result<SessionInfo> {
    let session_id = session_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("Invalid session directory: {:?}", session_dir))?;

    // Find highest plan iteration
    let (plan_iteration, title) = find_latest_plan(session_dir)?;

    // If no plan files, status is Ready
    if plan_iteration == 0 {
        return Ok(SessionInfo {
            session_id,
            session_dir: session_dir.to_path_buf(),
            iteration: 0,
            status: SessionStatus::Ready,
            latest_score: None,
            input_reason: None,
            title: None,
        });
    }

    // Find latest review
    let (review_iteration, review) = find_latest_review(session_dir)?;

    // Determine status based on review
    let (status, latest_score, input_reason) = match review {
        Some(review) => {
            let score = review.llm_review.score;

            if review.llm_review.requires_human_input {
                let reason = review.llm_review.human_input_reason.clone();
                (SessionStatus::NeedsInput, Some(score), reason)
            } else if score >= pass_threshold && !review.llm_review.requires_human_input {
                (SessionStatus::Approved, Some(score), None)
            } else if review_iteration >= max_iterations {
                (SessionStatus::MaxTurns, Some(score), None)
            } else {
                (SessionStatus::InProgress, Some(score), None)
            }
        }
        None => {
            // Has plan but no review yet
            (SessionStatus::InProgress, None, None)
        }
    };

    Ok(SessionInfo {
        session_id,
        session_dir: session_dir.to_path_buf(),
        iteration: plan_iteration.max(review_iteration),
        status,
        latest_score,
        input_reason,
        title,
    })
}

/// Find the latest plan iteration and extract the title
fn find_latest_plan(session_dir: &Path) -> anyhow::Result<(u32, Option<String>)> {
    let mut highest_iteration = 0u32;
    let mut latest_plan_path: Option<PathBuf> = None;

    if !session_dir.exists() {
        return Ok((0, None));
    }

    for entry in std::fs::read_dir(session_dir)? {
        let entry = entry?;
        let filename = entry.file_name();
        let filename_str = filename.to_string_lossy();

        if let Some(iter_str) = filename_str
            .strip_prefix("plan-iteration-")
            .and_then(|s| s.strip_suffix(".json"))
            && let Ok(iter) = iter_str.parse::<u32>()
            && iter > highest_iteration
        {
            highest_iteration = iter;
            latest_plan_path = Some(entry.path());
        }
    }

    // Extract title from latest plan if available
    let title = if let Some(path) = latest_plan_path {
        let content = std::fs::read_to_string(&path)?;
        let plan: serde_json::Value = serde_json::from_str(&content)?;
        plan.get("title").and_then(|v| v.as_str()).map(String::from)
    } else {
        None
    };

    Ok((highest_iteration, title))
}

/// Find the latest review iteration and parse it
fn find_latest_review(session_dir: &Path) -> anyhow::Result<(u32, Option<ReviewResult>)> {
    let mut highest_iteration = 0u32;
    let mut latest_review_path: Option<PathBuf> = None;

    if !session_dir.exists() {
        return Ok((0, None));
    }

    for entry in std::fs::read_dir(session_dir)? {
        let entry = entry?;
        let filename = entry.file_name();
        let filename_str = filename.to_string_lossy();

        if let Some(iter_str) = filename_str
            .strip_prefix("review-iteration-")
            .and_then(|s| s.strip_suffix(".json"))
            && let Ok(iter) = iter_str.parse::<u32>()
            && iter > highest_iteration
        {
            highest_iteration = iter;
            latest_review_path = Some(entry.path());
        }
    }

    // Parse latest review if available
    let review = if let Some(path) = latest_review_path {
        let content = std::fs::read_to_string(&path)?;
        Some(serde_json::from_str(&content)?)
    } else {
        None
    };

    Ok((highest_iteration, review))
}

/// List all sessions in the .plan-forge directory
pub fn list_sessions(plan_forge_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut sessions = Vec::new();

    if !plan_forge_dir.exists() {
        return Ok(sessions);
    }

    for entry in std::fs::read_dir(plan_forge_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip .goose directory and non-directories
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden directories (like .goose)
        if name_str.starts_with('.') {
            continue;
        }

        sessions.push(name_str.to_string());
    }

    // Sort by modification time (newest first)
    sessions.sort_by(|a, b| {
        let path_a = plan_forge_dir.join(a);
        let path_b = plan_forge_dir.join(b);

        let mtime_a = std::fs::metadata(&path_a).and_then(|m| m.modified()).ok();
        let mtime_b = std::fs::metadata(&path_b).and_then(|m| m.modified()).ok();

        mtime_b.cmp(&mtime_a)
    });

    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_derive_status_ready() {
        let temp = TempDir::new().unwrap();
        let session_dir = temp.path().join("my-session");
        fs::create_dir(&session_dir).unwrap();

        let info = derive_status(&session_dir, 0.8, 5).unwrap();
        assert_eq!(info.status, SessionStatus::Ready);
        assert_eq!(info.iteration, 0);
    }

    #[test]
    fn test_derive_status_in_progress() {
        let temp = TempDir::new().unwrap();
        let session_dir = temp.path().join("my-session");
        fs::create_dir(&session_dir).unwrap();

        // Create a plan file but no review
        let plan = serde_json::json!({
            "title": "Test Plan",
            "summary": "A test"
        });
        fs::write(
            session_dir.join("plan-iteration-1.json"),
            serde_json::to_string(&plan).unwrap(),
        )
        .unwrap();

        let info = derive_status(&session_dir, 0.8, 5).unwrap();
        assert_eq!(info.status, SessionStatus::InProgress);
        assert_eq!(info.iteration, 1);
        assert_eq!(info.title, Some("Test Plan".to_string()));
    }
}
