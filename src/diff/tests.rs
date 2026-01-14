//! Tests for diff parsing.

use super::api::AddedLine;
use super::helpers::parse_hunk_header;
use super::parser::parse_added_lines_from_diff;
use super::{added_lines, changed_files};

/// Test parsing a simple diff with one file and added lines.
#[test]
fn test_parse_simple_added_lines() {
    let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,0 +11,2 @@ fn existing_function() {
+    let x = 42;
+    println!("Added line");
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].file_path, "src/lib.rs");
    assert_eq!(result[0].line_number, 11);
    assert_eq!(result[0].content, "    let x = 42;");
    assert_eq!(result[1].line_number, 12);
    assert_eq!(result[1].content, "    println!(\"Added line\");");
}

/// Test parsing a new file (source is /dev/null).
#[test]
fn test_parse_new_file() {
    let diff = r#"diff --git a/src/new_file.rs b/src/new_file.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/src/new_file.rs
@@ -0,0 +1,3 @@
+//! New module
+
+pub fn hello() {}
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].file_path, "src/new_file.rs");
    assert_eq!(result[0].line_number, 1);
    assert_eq!(result[0].content, "//! New module");
    assert_eq!(result[1].line_number, 2);
    assert_eq!(result[1].content, "");
    assert_eq!(result[2].line_number, 3);
    assert_eq!(result[2].content, "pub fn hello() {}");
}

/// Test parsing multiple hunks in one file.
#[test]
fn test_parse_multiple_hunks() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -5,0 +6,1 @@ fn main() {
+    // First addition at line 6
@@ -20,0 +22,1 @@ fn helper() {
+    // Second addition at line 22
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].line_number, 6);
    assert_eq!(result[0].content, "    // First addition at line 6");
    assert_eq!(result[1].line_number, 22);
    assert_eq!(result[1].content, "    // Second addition at line 22");
}

/// Test parsing a hunk with both additions and deletions.
#[test]
fn test_parse_mixed_hunk() {
    let diff = r#"diff --git a/src/config.rs b/src/config.rs
index abc1234..def5678 100644
--- a/src/config.rs
+++ b/src/config.rs
@@ -10,2 +10,3 @@ struct Config {
-    old_field: i32,
-    another_old: String,
+    new_field: i64,
+    another_new: String,
+    extra_field: bool,
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    // Only added lines should be captured
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].file_path, "src/config.rs");
    assert_eq!(result[0].line_number, 10);
    assert_eq!(result[0].content, "    new_field: i64,");
    assert_eq!(result[1].line_number, 11);
    assert_eq!(result[1].content, "    another_new: String,");
    assert_eq!(result[2].line_number, 12);
    assert_eq!(result[2].content, "    extra_field: bool,");
}

/// Test that removed lines are not captured.
#[test]
fn test_removed_lines_not_captured() {
    let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -5,2 +5,0 @@ fn main() {
-    let x = 1;
-    let y = 2;
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    // No added lines
    assert!(result.is_empty());
}

/// Test parsing multiple files.
#[test]
fn test_parse_multiple_files() {
    let diff = r#"diff --git a/src/first.rs b/src/first.rs
index abc1234..def5678 100644
--- a/src/first.rs
+++ b/src/first.rs
@@ -1,0 +2,1 @@
+// Added to first.rs
diff --git a/src/second.rs b/src/second.rs
index 111111..222222 100644
--- a/src/second.rs
+++ b/src/second.rs
@@ -5,0 +6,1 @@
+// Added to second.rs
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].file_path, "src/first.rs");
    assert_eq!(result[0].line_number, 2);
    assert_eq!(result[0].content, "// Added to first.rs");
    assert_eq!(result[1].file_path, "src/second.rs");
    assert_eq!(result[1].line_number, 6);
    assert_eq!(result[1].content, "// Added to second.rs");
}

/// Test parsing a file rename.
#[test]
fn test_parse_rename() {
    let diff = r#"diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 95%
rename from src/old_name.rs
rename to src/new_name.rs
index abc1234..def5678 100644
--- a/src/old_name.rs
+++ b/src/new_name.rs
@@ -10,0 +11,1 @@
+// Added line in renamed file
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    assert_eq!(result.len(), 1);
    // Should use the new file name
    assert_eq!(result[0].file_path, "src/new_name.rs");
    assert_eq!(result[0].line_number, 11);
}

/// Test parsing hunk headers with various formats.
#[test]
fn test_parse_hunk_header_formats() {
    // Standard format with lengths
    assert_eq!(parse_hunk_header("@@ -10,5 +20,3 @@"), Some((10, 20)));

    // Without lengths (single line change)
    assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1)));

    // With context info after @@
    assert_eq!(
        parse_hunk_header("@@ -10,5 +20,3 @@ fn foo()"),
        Some((10, 20))
    );

    // Zero-length addition (insertion)
    assert_eq!(parse_hunk_header("@@ -5,0 +6,2 @@"), Some((5, 6)));

    // Line 0 (new file, no prior content)
    assert_eq!(parse_hunk_header("@@ -0,0 +1,10 @@"), Some((0, 1)));
}

/// Test normalize_path converts backslashes.
#[test]
fn test_normalize_path() {
    use super::helpers::normalize_path;
    assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
    assert_eq!(normalize_path("src\\lib.rs"), "src/lib.rs");
    assert_eq!(normalize_path("src\\nested\\file.rs"), "src/nested/file.rs");
}

/// Test empty diff returns empty results.
#[test]
fn test_empty_diff() {
    let result = parse_added_lines_from_diff("").unwrap();
    assert!(result.is_empty());
}

/// Test diff with only metadata lines (no actual changes).
#[test]
fn test_diff_metadata_only() {
    let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();
    assert!(result.is_empty());
}

/// Test comprehensive fixture combining multiple scenarios.
#[test]
fn test_comprehensive_fixture() {
    let diff = r#"diff --git a/src/player/jump.rs b/src/player/jump.rs
index abc1234..def5678 100644
--- a/src/player/jump.rs
+++ b/src/player/jump.rs
@@ -15,2 +15,3 @@ impl Player {
-    fn old_jump(&mut self) {
-        // old implementation
+    fn jump(&mut self) {
+        self.velocity.y = JUMP_FORCE;
+        // TODO: implement cooldown
@@ -30,0 +32,1 @@ impl Player {
+    unimplemented!()
diff --git a/src/player/mod.rs b/src/player/mod.rs
new file mode 100644
index 0000000..111111
--- /dev/null
+++ b/src/player/mod.rs
@@ -0,0 +1,5 @@
+//! Player module
+
+mod jump;
+
+pub use jump::Player;
diff --git a/src/config.rs b/src/config.rs
index 222222..333333 100644
--- a/src/config.rs
+++ b/src/config.rs
@@ -5,1 +5,1 @@ const MAX_PLAYERS: usize = 4;
-const JUMP_FORCE: f32 = 10.0;
+const JUMP_FORCE: f32 = 15.0;
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    // Count added lines per file
    let jump_lines: Vec<_> = result
        .iter()
        .filter(|l| l.file_path == "src/player/jump.rs")
        .collect();
    let mod_lines: Vec<_> = result
        .iter()
        .filter(|l| l.file_path == "src/player/mod.rs")
        .collect();
    let config_lines: Vec<_> = result
        .iter()
        .filter(|l| l.file_path == "src/config.rs")
        .collect();

    // jump.rs: 3 lines in first hunk, 1 in second
    assert_eq!(jump_lines.len(), 4);
    assert_eq!(jump_lines[0].line_number, 15);
    assert_eq!(jump_lines[0].content, "    fn jump(&mut self) {");
    assert_eq!(jump_lines[2].content, "        // TODO: implement cooldown");
    assert_eq!(jump_lines[3].line_number, 32);
    assert_eq!(jump_lines[3].content, "    unimplemented!()");

    // mod.rs: 5 lines (new file)
    assert_eq!(mod_lines.len(), 5);
    assert_eq!(mod_lines[0].line_number, 1);
    assert_eq!(mod_lines[0].content, "//! Player module");

    // config.rs: 1 added line (replacement)
    assert_eq!(config_lines.len(), 1);
    assert_eq!(config_lines[0].line_number, 5);
    assert_eq!(config_lines[0].content, "const JUMP_FORCE: f32 = 15.0;");
}

/// Test context lines are handled (rare with -U0, but should work).
#[test]
fn test_context_lines() {
    // With -U0 we shouldn't see context, but if present, handle it
    let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -5,3 +5,4 @@ fn main() {
 // context line
+// added line
 // another context
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    // The added line should be at line 6 (after context at 5)
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].line_number, 6);
    assert_eq!(result[0].content, "// added line");
}

/// Test deleted file (destination is /dev/null) produces no added lines.
#[test]
fn test_deleted_file() {
    let diff = r#"diff --git a/src/deleted.rs b/src/deleted.rs
deleted file mode 100644
index abc1234..0000000
--- a/src/deleted.rs
+++ /dev/null
@@ -1,5 +0,0 @@
-//! This file is deleted
-
-pub fn old_function() {
-    // old code
-}
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    // No added lines from deleted file
    assert!(result.is_empty());
}

/// Test file path with spaces.
#[test]
fn test_file_path_with_spaces() {
    let diff = r#"diff --git a/src/my file.rs b/src/my file.rs
index abc1234..def5678 100644
--- a/src/my file.rs
+++ b/src/my file.rs
@@ -1,0 +2,1 @@
+// Added line
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].file_path, "src/my file.rs");
}

/// Test binary file (should produce no added lines).
#[test]
fn test_binary_file() {
    let diff = r#"diff --git a/assets/image.png b/assets/image.png
new file mode 100644
index 0000000..abc1234
Binary files /dev/null and b/assets/image.png differ
"#;

    let result = parse_added_lines_from_diff(diff).unwrap();

    // Binary files don't have text content
    assert!(result.is_empty());
}

/// Test AddedLine struct equality.
#[test]
fn test_added_line_equality() {
    let line1 = AddedLine {
        file_path: "src/lib.rs".to_string(),
        line_number: 10,
        content: "let x = 1;".to_string(),
    };
    let line2 = AddedLine {
        file_path: "src/lib.rs".to_string(),
        line_number: 10,
        content: "let x = 1;".to_string(),
    };
    let line3 = AddedLine {
        file_path: "src/lib.rs".to_string(),
        line_number: 11,
        content: "let x = 1;".to_string(),
    };

    assert_eq!(line1, line2);
    assert_ne!(line1, line3);
}

/// Integration test: parse diff from git command.
/// This test requires a real git repo, so it uses tempfile.
#[test]
fn test_integration_with_git() {
    use std::process::Command;
    use tempfile::TempDir;

    // Create a temporary git repository
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();

    // Initialize git repo
    Command::new("git")
        .current_dir(path)
        .args(["init"])
        .output()
        .expect("failed to init git repo");

    // Configure git user
    Command::new("git")
        .current_dir(path)
        .args(["config", "user.email", "test@example.com"])
        .output()
        .expect("failed to set git email");

    Command::new("git")
        .current_dir(path)
        .args(["config", "user.name", "Test User"])
        .output()
        .expect("failed to set git name");

    // Create initial file and commit
    std::fs::write(path.join("test.rs"), "fn main() {}\n").unwrap();
    Command::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output()
        .expect("failed to add files");
    Command::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Initial commit"])
        .output()
        .expect("failed to commit");

    // Get the base SHA
    let base_output = Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("failed to get HEAD");
    let base_sha = String::from_utf8_lossy(&base_output.stdout)
        .trim()
        .to_string();

    // Modify the file and create new file
    std::fs::write(path.join("test.rs"), "fn main() {\n    let x = 42;\n}\n").unwrap();
    std::fs::write(path.join("new.rs"), "// New file\npub fn hello() {}\n").unwrap();
    Command::new("git")
        .current_dir(path)
        .args(["add", "."])
        .output()
        .expect("failed to add files");
    Command::new("git")
        .current_dir(path)
        .args(["commit", "-m", "Add changes"])
        .output()
        .expect("failed to commit");

    // Test changed_files
    let files = changed_files(path, &base_sha).unwrap();
    assert!(files.contains(&"test.rs".to_string()));
    assert!(files.contains(&"new.rs".to_string()));

    // Test added_lines
    let lines = added_lines(path, &base_sha).unwrap();

    // Should have lines from both files
    let test_lines: Vec<_> = lines.iter().filter(|l| l.file_path == "test.rs").collect();
    let new_lines: Vec<_> = lines.iter().filter(|l| l.file_path == "new.rs").collect();

    assert!(!test_lines.is_empty(), "Should have added lines in test.rs");
    assert!(!new_lines.is_empty(), "Should have added lines in new.rs");

    // Verify new.rs has expected content
    assert!(new_lines.iter().any(|l| l.content == "// New file"));
    assert!(new_lines.iter().any(|l| l.content == "pub fn hello() {}"));
}
