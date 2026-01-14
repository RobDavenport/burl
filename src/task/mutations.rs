//! Mutation helpers for common workflow operations.

use super::TaskFile;
use chrono::{DateTime, Utc};

impl TaskFile {
    /// Set the assigned_to field and optionally started_at timestamp.
    pub fn set_assigned(&mut self, assignee: &str, start_time: Option<DateTime<Utc>>) {
        self.frontmatter.assigned_to = Some(assignee.to_string());
        if let Some(time) = start_time {
            self.frontmatter.started_at = Some(time);
        }
    }

    /// Set git-related fields on claim.
    pub fn set_git_info(&mut self, branch: &str, worktree: &str, base_sha: &str) {
        self.frontmatter.branch = Some(branch.to_string());
        self.frontmatter.worktree = Some(worktree.to_string());
        self.frontmatter.base_sha = Some(base_sha.to_string());
    }

    /// Set the submitted_at timestamp.
    pub fn set_submitted(&mut self, time: DateTime<Utc>) {
        self.frontmatter.submitted_at = Some(time);
    }

    /// Set the completed_at timestamp.
    pub fn set_completed(&mut self, time: DateTime<Utc>) {
        self.frontmatter.completed_at = Some(time);
    }

    /// Increment the qa_attempts counter.
    pub fn increment_qa_attempts(&mut self) {
        self.frontmatter.qa_attempts += 1;
    }

    /// Append content to the QA Report section.
    ///
    /// If the section exists, content is appended below it.
    /// If not, a new section is created at the end of the body.
    pub fn append_to_qa_report(&mut self, content: &str) {
        const QA_REPORT_HEADING: &str = "## QA Report";

        // Ensure body ends with newline for clean appending
        if !self.body.is_empty() && !self.body.ends_with('\n') {
            self.body.push('\n');
        }

        if let Some(pos) = self.body.find(QA_REPORT_HEADING) {
            // Find the end of the QA Report section (next ## heading or end of body)
            let after_heading = pos + QA_REPORT_HEADING.len();
            let section_end = self.body[after_heading..]
                .find("\n## ")
                .map(|p| after_heading + p)
                .unwrap_or(self.body.len());

            // Insert content at the end of the section
            let insert_pos = section_end;

            // Ensure there's a newline before the new content
            let prefix = if insert_pos > 0 && !self.body[..insert_pos].ends_with('\n') {
                "\n"
            } else {
                ""
            };

            self.body
                .insert_str(insert_pos, &format!("{}{}\n", prefix, content));
        } else {
            // Create new QA Report section at the end
            self.body
                .push_str(&format!("\n{}\n{}\n", QA_REPORT_HEADING, content));
        }
    }

    /// Clear the assigned_to field (for unassignment).
    pub fn clear_assigned(&mut self) {
        self.frontmatter.assigned_to = None;
    }
}
