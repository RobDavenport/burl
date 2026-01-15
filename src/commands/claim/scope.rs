//! Scope conflict detection for claim operation.

use crate::config::{ConflictDetectionMode, ConflictPolicy};
use crate::diff;
use crate::error::{BurlError, Result};
use crate::task::TaskFile;
use crate::workflow::TaskIndex;

/// Check if two scopes overlap.
///
/// Overlap detection rules:
/// - overlap if any explicit `affects` path in task A matches any `affects_globs` pattern in task B (and vice versa)
/// - overlap if any explicit `affects` path is identical between tasks
/// - overlap if any `affects_globs` pattern is identical between tasks
/// - treat prefix relationships as overlap for directory globs (e.g., `src/**` overlaps `src/foo/**`)
pub fn scopes_overlap(
    task_a_affects: &[String],
    task_a_globs: &[String],
    task_b_affects: &[String],
    task_b_globs: &[String],
) -> bool {
    // Check identical affects paths
    for path_a in task_a_affects {
        if task_b_affects.contains(path_a) {
            return true;
        }
    }

    // Check identical glob patterns
    for glob_a in task_a_globs {
        if task_b_globs.contains(glob_a) {
            return true;
        }
    }

    // Check if any affects path matches a glob pattern (conservative heuristic)
    for path in task_a_affects {
        for glob in task_b_globs {
            if path_matches_glob_heuristic(path, glob) {
                return true;
            }
        }
    }

    for path in task_b_affects {
        for glob in task_a_globs {
            if path_matches_glob_heuristic(path, glob) {
                return true;
            }
        }
    }

    // Check directory glob prefix relationships
    for glob_a in task_a_globs {
        for glob_b in task_b_globs {
            if globs_overlap_heuristic(glob_a, glob_b) {
                return true;
            }
        }
    }

    false
}

/// Conservative heuristic to check if a path might match a glob pattern.
///
/// This is a simple prefix/suffix check, not a full glob matcher.
fn path_matches_glob_heuristic(path: &str, glob: &str) -> bool {
    // Normalize paths for comparison
    let path_normalized = path.replace('\\', "/");
    let glob_normalized = glob.replace('\\', "/");

    // Handle common glob patterns
    if let Some(prefix) = glob_normalized.strip_suffix("/**") {
        // Directory glob: src/** matches anything under src/
        if path_normalized.starts_with(prefix)
            || path_normalized.starts_with(&format!("{}/", prefix))
        {
            return true;
        }
    }

    if let Some(prefix) = glob_normalized.strip_suffix("/*") {
        // Single-level glob: src/* matches direct children
        if let Some(path_prefix) = path_normalized.rsplit_once('/')
            && path_prefix.0 == prefix
        {
            return true;
        }
    }

    // Exact prefix match for simple cases
    if path_normalized.starts_with(&glob_normalized.replace("**", ""))
        || path_normalized.starts_with(&glob_normalized.replace("*", ""))
    {
        return true;
    }

    false
}

/// Check if two globs have overlapping coverage.
fn globs_overlap_heuristic(glob_a: &str, glob_b: &str) -> bool {
    let a_normalized = glob_a.replace('\\', "/");
    let b_normalized = glob_b.replace('\\', "/");

    // Extract the base directory from globs like "src/foo/**"
    let a_base = a_normalized
        .strip_suffix("/**")
        .or_else(|| a_normalized.strip_suffix("/*"))
        .unwrap_or(&a_normalized);

    let b_base = b_normalized
        .strip_suffix("/**")
        .or_else(|| b_normalized.strip_suffix("/*"))
        .unwrap_or(&b_normalized);

    // Check if one is a prefix of the other
    a_base.starts_with(b_base) || b_base.starts_with(a_base)
}

/// Check for scope conflicts with tasks currently in DOING.
pub fn check_scope_conflicts(
    ctx: &crate::context::WorkflowContext,
    task: &TaskFile,
    index: &TaskIndex,
    detection: ConflictDetectionMode,
    policy: ConflictPolicy,
) -> Result<()> {
    if policy == ConflictPolicy::Ignore {
        return Ok(());
    }

    let doing_tasks = index.tasks_in_bucket("DOING");
    let mut conflicts: Vec<String> = Vec::new();

    let claiming_affects = &task.frontmatter.affects;
    let claiming_globs = &task.frontmatter.affects_globs;

    for doing_task in doing_tasks {
        let doing_file = TaskFile::load(&doing_task.path)?;

        let declared_overlap = scopes_overlap(
            claiming_affects,
            claiming_globs,
            &doing_file.frontmatter.affects,
            &doing_file.frontmatter.affects_globs,
        );

        let diff_overlap = match detection {
            ConflictDetectionMode::Declared => None,
            ConflictDetectionMode::Diff | ConflictDetectionMode::Hybrid => {
                diff_overlap_with_doing_task(ctx, task, &doing_task.id, &doing_file)
            }
        };

        let overlaps = match detection {
            ConflictDetectionMode::Declared => declared_overlap,
            ConflictDetectionMode::Diff => diff_overlap.unwrap_or(declared_overlap),
            ConflictDetectionMode::Hybrid => {
                // Prefer actual changes when available; fallback to declared scopes.
                match diff_overlap {
                    Some(overlap)
                        if !doing_file_has_empty_diff(ctx, &doing_task.id, &doing_file) =>
                    {
                        overlap
                    }
                    Some(_) => declared_overlap,
                    None => declared_overlap,
                }
            }
        };

        if overlaps {
            conflicts.push(format!(
                "{} ({})",
                doing_task.id, doing_file.frontmatter.title
            ));
        }
    }

    if conflicts.is_empty() {
        return Ok(());
    }

    let conflict_msg = format!(
        "scope conflict detected with tasks currently in DOING:\n  - {}\n\n\
         The declaring scopes overlap, which may cause merge conflicts.",
        conflicts.join("\n  - ")
    );

    match policy {
        ConflictPolicy::Fail => Err(BurlError::UserError(format!(
            "cannot claim task: {}\n\n\
             To proceed anyway, set `conflict_policy: warn` or `conflict_policy: ignore` in config.yaml.",
            conflict_msg
        ))),
        ConflictPolicy::Warn => {
            eprintln!("Warning: {}", conflict_msg);
            Ok(())
        }
        ConflictPolicy::Ignore => Ok(()),
    }
}

fn diff_overlap_with_doing_task(
    ctx: &crate::context::WorkflowContext,
    claiming_task: &TaskFile,
    doing_task_id: &str,
    doing_task: &TaskFile,
) -> Option<bool> {
    let base_sha = doing_task.frontmatter.base_sha.as_deref()?;

    let refs = crate::task_git::validate_task_git_refs_if_present(
        ctx,
        doing_task_id,
        doing_task.frontmatter.branch.as_deref(),
        doing_task.frontmatter.worktree.as_deref(),
    )
    .ok()??;

    if !refs.worktree_path.exists() {
        return None;
    }

    let changed = diff::changed_files(&refs.worktree_path, base_sha).ok()?;
    Some(any_file_in_scope(
        &claiming_task.frontmatter.affects,
        &claiming_task.frontmatter.affects_globs,
        &changed,
    ))
}

fn doing_file_has_empty_diff(
    ctx: &crate::context::WorkflowContext,
    doing_task_id: &str,
    doing_task: &TaskFile,
) -> bool {
    let Some(base_sha) = doing_task.frontmatter.base_sha.as_deref() else {
        return true;
    };

    let refs = match crate::task_git::validate_task_git_refs_if_present(
        ctx,
        doing_task_id,
        doing_task.frontmatter.branch.as_deref(),
        doing_task.frontmatter.worktree.as_deref(),
    ) {
        Ok(Some(r)) => r,
        _ => return true,
    };

    if !refs.worktree_path.exists() {
        return true;
    }

    diff::changed_files(&refs.worktree_path, base_sha)
        .map(|files| files.is_empty())
        .unwrap_or(true)
}

fn any_file_in_scope(affects: &[String], globs: &[String], changed_files: &[String]) -> bool {
    if changed_files.is_empty() {
        return false;
    }

    let allowed_paths: std::collections::HashSet<String> =
        affects.iter().map(|p| p.replace('\\', "/")).collect();

    let mut builder = globset::GlobSetBuilder::new();
    for pattern in globs {
        let normalized = pattern.replace('\\', "/");
        if let Ok(glob) = globset::Glob::new(&normalized) {
            builder.add(glob);
        }
    }
    let globset = builder.build().ok();

    changed_files.iter().any(|file| {
        let file = file.replace('\\', "/");
        if allowed_paths.contains(&file) {
            return true;
        }

        if let Some(ref set) = globset
            && set.is_match(&file)
        {
            return true;
        }

        // Treat explicit `affects` paths as directories when appropriate (mirrors scope validator).
        for allowed in &allowed_paths {
            if allowed.ends_with('/') && file.starts_with(allowed) {
                return true;
            }

            let dir_prefix = if allowed.ends_with('/') {
                allowed.clone()
            } else {
                format!("{}/", allowed)
            };
            if file.starts_with(&dir_prefix) {
                return true;
            }
        }

        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scopes_overlap_identical_affects() {
        let affects_a = vec!["src/lib.rs".to_string()];
        let globs_a = vec![];
        let affects_b = vec!["src/lib.rs".to_string()];
        let globs_b = vec![];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_overlap_identical_globs() {
        let affects_a = vec![];
        let globs_a = vec!["src/**".to_string()];
        let affects_b = vec![];
        let globs_b = vec!["src/**".to_string()];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_overlap_path_matches_glob() {
        let affects_a = vec!["src/lib.rs".to_string()];
        let globs_a = vec![];
        let affects_b = vec![];
        let globs_b = vec!["src/**".to_string()];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_no_overlap() {
        let affects_a = vec!["src/lib.rs".to_string()];
        let globs_a = vec!["src/**".to_string()];
        let affects_b = vec!["tests/test.rs".to_string()];
        let globs_b = vec!["tests/**".to_string()];

        assert!(!scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_scopes_overlap_nested_globs() {
        let affects_a = vec![];
        let globs_a = vec!["src/**".to_string()];
        let affects_b = vec![];
        let globs_b = vec!["src/foo/**".to_string()];

        assert!(scopes_overlap(&affects_a, &globs_a, &affects_b, &globs_b));
    }

    #[test]
    fn test_path_matches_glob_heuristic() {
        assert!(path_matches_glob_heuristic("src/lib.rs", "src/**"));
        assert!(path_matches_glob_heuristic("src/foo/bar.rs", "src/**"));
        assert!(path_matches_glob_heuristic("src/lib.rs", "src/*"));
        assert!(!path_matches_glob_heuristic("tests/test.rs", "src/**"));
    }

    #[test]
    fn test_globs_overlap_heuristic() {
        assert!(globs_overlap_heuristic("src/**", "src/foo/**"));
        assert!(globs_overlap_heuristic("src/foo/**", "src/**"));
        assert!(!globs_overlap_heuristic("src/**", "tests/**"));
    }
}
