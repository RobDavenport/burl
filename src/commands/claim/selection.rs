//! Task selection and dependency checking for claim operation.

use crate::error::{BurlError, Result};
use crate::task::TaskFile;
use crate::workflow::{TaskIndex, TaskInfo};

/// Priority ordering for task selection (high > medium > low > none/other)
fn priority_rank(priority: &str) -> u32 {
    match priority.to_lowercase().as_str() {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    }
}

/// Select the next claimable task deterministically.
///
/// Selection criteria:
/// 1. Task must be in READY bucket
/// 2. Sort by priority (high > medium > low > none)
/// 3. Then by numeric ID ascending
///
/// Returns the task ID of the selected task.
pub fn select_next_task_id(
    ctx: &crate::context::WorkflowContext,
    ready_tasks: &[&TaskInfo],
) -> Result<Option<String>> {
    if ready_tasks.is_empty() {
        return Ok(None);
    }

    // Load task files to get priority info - collect into owned data
    let mut tasks_with_priority: Vec<(String, u32, u32)> = Vec::new(); // (id, priority_rank, number)

    for task_info in ready_tasks {
        let task_file = TaskFile::load(&task_info.path)?;
        let rank = priority_rank(&task_file.frontmatter.priority);
        tasks_with_priority.push((task_info.id.clone(), rank, task_info.number));
    }

    // Sort by priority rank (ascending = higher priority first), then by numeric ID
    tasks_with_priority.sort_by(|a, b| {
        let priority_cmp = a.1.cmp(&b.1);
        if priority_cmp == std::cmp::Ordering::Equal {
            a.2.cmp(&b.2)
        } else {
            priority_cmp
        }
    });

    // Filter out tasks with unmet dependencies
    for (task_id, _, _) in tasks_with_priority {
        // Re-fetch task info from a fresh index
        let index = TaskIndex::build(ctx)?;
        if let Some(task_info) = index.find(&task_id) {
            let task_file = TaskFile::load(&task_info.path)?;
            if check_dependencies_satisfied(&task_file, &index).is_ok() {
                return Ok(Some(task_id));
            }
        }
    }

    Ok(None)
}

/// Check if all dependencies of a task are in DONE.
pub fn check_dependencies_satisfied(task: &TaskFile, index: &TaskIndex) -> Result<()> {
    let mut unmet_deps = Vec::new();

    for dep_id in &task.frontmatter.depends_on {
        match index.find(dep_id) {
            Some(dep_info) => {
                if dep_info.bucket != "DONE" {
                    unmet_deps.push(format!("{} (currently in {})", dep_id, dep_info.bucket));
                }
            }
            None => {
                unmet_deps.push(format!("{} (not found)", dep_id));
            }
        }
    }

    if !unmet_deps.is_empty() {
        return Err(BurlError::UserError(format!(
            "cannot claim task: dependencies not satisfied.\n\n\
             Unmet dependencies:\n  - {}\n\n\
             Complete these tasks first before claiming this one.",
            unmet_deps.join("\n  - ")
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_rank() {
        assert_eq!(priority_rank("high"), 0);
        assert_eq!(priority_rank("HIGH"), 0);
        assert_eq!(priority_rank("medium"), 1);
        assert_eq!(priority_rank("low"), 2);
        assert_eq!(priority_rank("other"), 3);
        assert_eq!(priority_rank(""), 3);
    }
}
