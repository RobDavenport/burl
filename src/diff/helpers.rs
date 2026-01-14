//! Helper functions for diff parsing.

/// Parse the file path from a "diff --git" line.
///
/// Handles various formats:
/// - "a/path/to/file b/path/to/file" (normal)
/// - "a/path/to/file b/path/to/renamed" (rename)
/// - "a/path b/path" (short paths)
///
/// Returns the "b/" path (new file path), or None if parsing fails.
pub(super) fn parse_diff_git_line(rest: &str) -> Option<String> {
    // The format is: "a/<path> b/<path>"
    // But paths can contain spaces, so we need to be careful
    // Strategy: find " b/" which separates the two paths

    // Handle the case where the path might contain " b/" as part of the path
    // by looking for the last " b/" occurrence
    if let Some(b_pos) = rest.rfind(" b/") {
        let b_path = &rest[b_pos + 3..]; // Skip " b/"
        return Some(normalize_path(b_path));
    }

    // Fallback: try to split on space and take the second part
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        let b_part = parts[parts.len() - 1];
        if let Some(path) = b_part.strip_prefix("b/") {
            return Some(normalize_path(path));
        }
    }

    None
}

/// Parse a hunk header line.
///
/// Format: "@@ -old_start,old_len +new_start,new_len @@" or "@@ -old_start +new_start @@"
/// Also handles: "@@ -old_start,old_len +new_start,new_len @@ context info"
///
/// Returns (old_start, new_start) or None if parsing fails.
pub(super) fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    // Remove leading "@@ " and trailing " @@" (with optional context)
    let line = line.strip_prefix("@@ ")?;

    // Find the closing " @@"
    let end_marker = line.find(" @@")?;
    let range_part = &line[..end_marker];

    // Split into old and new ranges
    let parts: Vec<&str> = range_part.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let old_part = parts[0].strip_prefix('-')?;
    let new_part = parts[1].strip_prefix('+')?;

    let old_start = parse_range_start(old_part)?;
    let new_start = parse_range_start(new_part)?;

    Some((old_start, new_start))
}

/// Parse the start line from a range specification.
///
/// Format: "start" or "start,len"
/// Returns the start line number.
fn parse_range_start(range: &str) -> Option<usize> {
    let start_str = if let Some(comma_pos) = range.find(',') {
        &range[..comma_pos]
    } else {
        range
    };

    start_str.parse().ok()
}

/// Normalize a file path to use forward slashes.
///
/// This ensures consistent path format for glob matching,
/// regardless of the platform where the diff was generated.
pub(super) fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}
