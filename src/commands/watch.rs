//! Implementation of the `burl watch` command.
//!
//! `watch` provides a simple automation loop to:
//! - keep claiming READY tasks up to `config.max_parallel`
//! - process QA tasks (validate, or approve if `--approve` is set)
//!
//! To avoid spamming repeated QA report entries, `watch` tracks the last-seen
//! HEAD SHA per QA task and only re-processes a task when its HEAD changes.

use crate::cli::{ApproveArgs, ClaimArgs, ValidateArgs, WatchArgs};
use crate::commands::{approve, claim, validate_cmd};
use crate::config::Config;
use crate::context::{WorkflowContext, require_initialized_workflow};
use crate::error::{BurlError, Result};
use crate::fs::atomic_write_file;
use crate::git::run_git;
use crate::task::TaskFile;
use crate::workflow::TaskIndex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const WATCH_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WatchState {
    version: u32,
    qa_head_sha: HashMap<String, String>,
}

impl Default for WatchState {
    fn default() -> Self {
        Self {
            version: WATCH_STATE_VERSION,
            qa_head_sha: HashMap::new(),
        }
    }
}

pub fn cmd_watch(args: WatchArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    let state_path = watch_state_path(&ctx);
    let mut state = load_watch_state(&state_path);

    eprintln!("burl watch started");
    eprintln!("  repo:     {}", ctx.repo_root.display());
    eprintln!("  workflow: {}", ctx.workflow_worktree.display());
    eprintln!(
        "  modes:    claim={} qa={} approve={}",
        args.claim, args.qa, args.approve
    );
    eprintln!("  interval: {}ms", args.interval_ms);
    eprintln!();

    loop {
        let mut changed_state = false;

        // Prune state to only current QA tasks (keeps file small over time).
        let current_qa_ids = current_qa_task_ids(&ctx)?;
        if prune_state(&mut state, &current_qa_ids) {
            changed_state = true;
        }

        if args.claim {
            // Claim changes are durable via command implementations; no watch-state update.
            let _ = claim_up_to_max_parallel(&ctx, &config)?;
        }

        if args.qa && process_qa_tasks(&ctx, &args, &mut state)? {
            changed_state = true;
        }

        if changed_state {
            save_watch_state(&state_path, &state)?;
        }

        if args.once {
            break;
        }

        thread::sleep(Duration::from_millis(args.interval_ms.max(50)));
    }

    Ok(())
}

fn watch_state_path(ctx: &WorkflowContext) -> PathBuf {
    ctx.locks_dir.join("watch.state.json")
}

fn load_watch_state(path: &Path) -> WatchState {
    let Ok(content) = std::fs::read_to_string(path) else {
        return WatchState::default();
    };

    match serde_json::from_str::<WatchState>(&content) {
        Ok(state) if state.version == WATCH_STATE_VERSION => state,
        Ok(_) => {
            eprintln!(
                "Warning: watch state file has unknown version; resetting: {}",
                path.display()
            );
            WatchState::default()
        }
        Err(e) => {
            eprintln!(
                "Warning: failed to parse watch state file; resetting: {} ({})",
                path.display(),
                e
            );
            WatchState::default()
        }
    }
}

fn save_watch_state(path: &Path, state: &WatchState) -> Result<()> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| BurlError::UserError(format!("failed to serialize watch state: {}", e)))?;
    atomic_write_file(path, &json)?;
    Ok(())
}

fn current_qa_task_ids(ctx: &WorkflowContext) -> Result<HashSet<String>> {
    let index = TaskIndex::build(ctx)?;
    Ok(index
        .tasks_in_bucket("QA")
        .into_iter()
        .map(|t| t.id.clone())
        .collect())
}

fn prune_state(state: &mut WatchState, qa_ids: &HashSet<String>) -> bool {
    let before = state.qa_head_sha.len();
    state.qa_head_sha.retain(|id, _| qa_ids.contains(id));
    state.qa_head_sha.len() != before
}

fn claim_up_to_max_parallel(ctx: &WorkflowContext, config: &Config) -> Result<bool> {
    let mut did_claim = false;

    loop {
        let index = TaskIndex::build(ctx)?;
        let doing = index.tasks_in_bucket("DOING").len();
        let target = config.max_parallel as usize;

        if doing >= target {
            break;
        }

        match claim::cmd_claim(ClaimArgs { task_id: None }) {
            Ok(()) => {
                did_claim = true;
                continue;
            }
            Err(BurlError::UserError(msg))
                if msg.contains("no claimable tasks in READY")
                    || msg.contains("no READY tasks") =>
            {
                break;
            }
            Err(e) => {
                eprintln!("watch: claim failed: {}", e);
                break;
            }
        }
    }

    Ok(did_claim)
}

fn process_qa_tasks(
    ctx: &WorkflowContext,
    args: &WatchArgs,
    state: &mut WatchState,
) -> Result<bool> {
    let index = TaskIndex::build(ctx)?;
    let qa_tasks = index.tasks_in_bucket("QA");
    if qa_tasks.is_empty() {
        return Ok(false);
    }

    let mut changed_state = false;

    for task_info in qa_tasks {
        let task_id = task_info.id.clone();

        // Load task file to find worktree path.
        let task_file = match TaskFile::load(&task_info.path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("watch: failed to load {}: {}", task_id, e);
                continue;
            }
        };

        let Some(worktree) = task_file.frontmatter.worktree.as_ref() else {
            eprintln!("watch: {} missing worktree; run `burl doctor`", task_id);
            continue;
        };

        let worktree_path = resolve_worktree_path(ctx, worktree);
        if !worktree_path.exists() {
            eprintln!(
                "watch: {} worktree missing at {}; run `burl doctor`",
                task_id,
                worktree_path.display()
            );
            continue;
        }

        let head_sha = match run_git(&worktree_path, &["rev-parse", "HEAD"]) {
            Ok(out) => out.stdout,
            Err(e) => {
                eprintln!("watch: failed to read HEAD for {}: {}", task_id, e);
                continue;
            }
        };

        if state
            .qa_head_sha
            .get(&task_id)
            .is_some_and(|s| s == &head_sha)
        {
            continue;
        }

        // Mark this SHA as processed regardless of the outcome to avoid re-running
        // validations on the same commit over and over.
        state.qa_head_sha.insert(task_id.clone(), head_sha);
        changed_state = true;

        if args.approve {
            eprintln!("watch: approving {}", task_id);
            if let Err(e) = approve::cmd_approve(ApproveArgs {
                task_id: task_id.clone(),
            }) {
                eprintln!("watch: approve failed for {}: {}", task_id, e);
            }
        } else {
            eprintln!("watch: validating {}", task_id);
            if let Err(e) = validate_cmd::cmd_validate(ValidateArgs {
                task_id: task_id.clone(),
            }) {
                // Validation failures are expected; keep going.
                eprintln!("watch: validate failed for {}: {}", task_id, e);
            }
        }
    }

    Ok(changed_state)
}

fn resolve_worktree_path(ctx: &WorkflowContext, worktree: &str) -> PathBuf {
    let path = PathBuf::from(worktree);
    if path.is_absolute() {
        path
    } else {
        ctx.repo_root.join(path)
    }
}
