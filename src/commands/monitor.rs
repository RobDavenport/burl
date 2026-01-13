//! Implementation of the `burl monitor` (visualizer) command.
//!
//! This is a lightweight, refresh-based dashboard intended to be cross-platform
//! without pulling in a full TUI dependency stack. It uses ANSI escape codes
//! to clear the screen between refreshes.

use crate::cli::MonitorArgs;
use crate::config::Config;
use crate::context::require_initialized_workflow;
use crate::error::Result;
use crate::events::Event;
use crate::locks;
use crate::task::TaskFile;
use crate::workflow::{BUCKETS, TaskIndex};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::cmp::Ordering;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

const DOING_STALL_HOURS: i64 = 24;
const QA_STALL_HOURS: i64 = 24;

#[derive(Debug, Clone)]
struct TaskRow {
    id: String,
    title: String,
    priority: String,
    assigned_to: Option<String>,
    qa_attempts: u32,
    started_at: Option<DateTime<Utc>>,
    submitted_at: Option<DateTime<Utc>>,
}

pub fn cmd_monitor(args: MonitorArgs) -> Result<()> {
    let ctx = require_initialized_workflow()?;
    let config = Config::load(ctx.config_path()).unwrap_or_default();

    loop {
        if args.clear {
            clear_screen();
        }

        render_once(&ctx, &config, &args)?;

        if args.once {
            break;
        }

        thread::sleep(Duration::from_millis(args.interval_ms.max(50)));
    }

    Ok(())
}

fn render_once(
    ctx: &crate::context::WorkflowContext,
    config: &Config,
    args: &MonitorArgs,
) -> Result<()> {
    let now = Utc::now();

    let index = TaskIndex::build(ctx)?;
    let bucket_counts = index.bucket_counts();
    let active_locks = locks::list_locks(ctx, config)?;

    println!("Burl Monitor  (Ctrl+C to exit)");
    println!("Updated: {}", now.format("%Y-%m-%d %H:%M:%S UTC"));
    println!();
    println!("Repo:     {}", ctx.repo_root.display());
    println!("Workflow: {}", ctx.workflow_worktree.display());
    println!();

    // Buckets
    println!("Buckets:");
    let total: usize = bucket_counts.values().sum();
    for bucket in BUCKETS {
        let count = bucket_counts.get(*bucket).copied().unwrap_or(0);
        print!("  {:8} {:>3}", bucket, count);

        // Add quick highlights
        if *bucket == "DOING" {
            let stalled = count_stalled(&index, "DOING", DOING_STALL_HOURS);
            if stalled > 0 {
                print!("  ({} stalled)", stalled);
            }
        }
        if *bucket == "QA" {
            let stalled = count_stalled(&index, "QA", QA_STALL_HOURS);
            if stalled > 0 {
                print!("  ({} stalled)", stalled);
            }
        }
        println!();
    }
    println!("  --------");
    println!("  {:8} {:>3}", "Total", total);
    println!();

    // Locks
    if !active_locks.is_empty() {
        let stale_count = active_locks.iter().filter(|l| l.is_stale).count();
        println!(
            "Locks: {}{}",
            active_locks.len(),
            if stale_count > 0 {
                format!("  ({} stale)", stale_count)
            } else {
                String::new()
            }
        );
        for lock in &active_locks {
            let stale_marker = if lock.is_stale { " [STALE]" } else { "" };
            println!(
                "  - {} ({}, {}, action: {}){}",
                lock.name,
                lock.metadata.owner,
                lock.metadata.age_string(),
                lock.metadata.action,
                stale_marker
            );
        }
        println!();
    }

    // DOING tasks
    render_bucket_tasks(
        "DOING",
        &index,
        args.limit,
        |a, b| match (a.started_at, b.started_at) {
            (Some(a_ts), Some(b_ts)) => a_ts.cmp(&b_ts),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        },
        now,
        |t| {
            let age = t
                .started_at
                .map(|ts| format_age(now, ts))
                .unwrap_or_else(|| "?".to_string());
            let assignee = t.assigned_to.as_deref().unwrap_or("-");
            format!(
                "{}  [{}]  started {}  {}  {}",
                t.id,
                t.priority,
                age,
                assignee,
                truncate_title(&t.title, 80)
            )
        },
    )?;

    // QA tasks
    render_bucket_tasks(
        "QA",
        &index,
        args.limit,
        |a, b| match (a.submitted_at, b.submitted_at) {
            (Some(a_ts), Some(b_ts)) => a_ts.cmp(&b_ts),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        },
        now,
        |t| {
            let age = t
                .submitted_at
                .map(|ts| format_age(now, ts))
                .unwrap_or_else(|| "?".to_string());
            let attempts = format!("{}/{}", t.qa_attempts, config.qa_max_attempts);
            let assignee = t.assigned_to.as_deref().unwrap_or("-");
            format!(
                "{}  [attempts {}]  submitted {}  {}  {}",
                t.id,
                attempts,
                age,
                assignee,
                truncate_title(&t.title, 80)
            )
        },
    )?;

    // Recent events
    if args.tail > 0 {
        let events = read_last_events(&ctx.events_file(), args.tail);
        if !events.is_empty() {
            println!("Recent events (last {}):", events.len());
            for event in events {
                let task = event.task.as_deref().unwrap_or("-");
                let mut extra = String::new();
                if let Some(passed) = event.details.get("passed").and_then(|v| v.as_bool()) {
                    extra = format!(" {}", if passed { "PASS" } else { "FAIL" });
                }
                println!(
                    "  - {}  {:10}  {:10}{}",
                    event.ts.format("%H:%M:%S"),
                    event.action,
                    task,
                    extra
                );
            }
            println!();
        }
    }

    // Helpful hint
    if total == 0 {
        println!("No tasks in the workflow. Run `burl add \"title\"` to create a task.");
        println!();
    }

    io::stdout().flush().ok();
    Ok(())
}

fn render_bucket_tasks<FSort, FLine>(
    bucket: &str,
    index: &TaskIndex,
    limit: usize,
    sort: FSort,
    now: DateTime<Utc>,
    line: FLine,
) -> Result<()>
where
    FSort: Fn(&TaskRow, &TaskRow) -> Ordering,
    FLine: Fn(&TaskRow) -> String,
{
    let tasks = index.tasks_in_bucket(bucket);
    if tasks.is_empty() {
        return Ok(());
    }

    let mut rows: Vec<TaskRow> = Vec::new();
    for info in tasks {
        match TaskFile::load(&info.path) {
            Ok(task_file) => rows.push(TaskRow {
                id: info.id.clone(),
                title: task_file.frontmatter.title,
                priority: task_file.frontmatter.priority,
                assigned_to: task_file.frontmatter.assigned_to,
                qa_attempts: task_file.frontmatter.qa_attempts,
                started_at: task_file.frontmatter.started_at,
                submitted_at: task_file.frontmatter.submitted_at,
            }),
            Err(_) => rows.push(TaskRow {
                id: info.id.clone(),
                title: "(failed to load task file)".to_string(),
                priority: "?".to_string(),
                assigned_to: None,
                qa_attempts: 0,
                started_at: None,
                submitted_at: None,
            }),
        }
    }

    rows.sort_by(sort);

    let shown = rows.len().min(limit);
    println!("{} ({}):", bucket, rows.len());
    for row in rows.iter().take(shown) {
        let mut prefix = "  - ";
        if bucket == "DOING"
            && row.started_at.is_some_and(|ts| {
                now.signed_duration_since(ts) > ChronoDuration::hours(DOING_STALL_HOURS)
            })
        {
            prefix = "  ! ";
        }
        if bucket == "QA"
            && row.submitted_at.is_some_and(|ts| {
                now.signed_duration_since(ts) > ChronoDuration::hours(QA_STALL_HOURS)
            })
        {
            prefix = "  ! ";
        }
        println!("{}{}", prefix, line(row));
    }
    if rows.len() > shown {
        println!("  ... and {} more", rows.len() - shown);
    }
    println!();

    Ok(())
}

fn count_stalled(index: &TaskIndex, bucket: &str, stall_hours: i64) -> usize {
    let now = Utc::now();
    let threshold = ChronoDuration::hours(stall_hours);

    index
        .tasks_in_bucket(bucket)
        .iter()
        .filter_map(|info| TaskFile::load(&info.path).ok())
        .filter(|task| match bucket {
            "DOING" => task
                .frontmatter
                .started_at
                .is_some_and(|ts| now.signed_duration_since(ts) > threshold),
            "QA" => task
                .frontmatter
                .submitted_at
                .is_some_and(|ts| now.signed_duration_since(ts) > threshold),
            _ => false,
        })
        .count()
}

fn format_age(now: DateTime<Utc>, ts: DateTime<Utc>) -> String {
    let age = now.signed_duration_since(ts);
    if age.num_days() > 0 {
        format!("{}d{}h", age.num_days(), age.num_hours() % 24)
    } else if age.num_hours() > 0 {
        format!("{}h{}m", age.num_hours(), age.num_minutes() % 60)
    } else {
        format!("{}m", age.num_minutes().max(0))
    }
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    if title.chars().count() <= max_chars {
        return title.to_string();
    }

    let take = max_chars.saturating_sub(3);
    let mut truncated: String = title.chars().take(take).collect();
    truncated.push_str("...");
    truncated
}

fn read_last_events(path: &Path, count: usize) -> Vec<Event> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut parsed: Vec<Event> = Vec::new();
    for line in content.lines().rev().take(count) {
        if let Ok(event) = serde_json::from_str::<Event>(line) {
            parsed.push(event);
        }
    }
    parsed.reverse();
    parsed
}

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = io::stdout().flush();
}
