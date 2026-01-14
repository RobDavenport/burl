//! Core types for stub validation results and violations.

/// A single stub violation found in an added line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubViolation {
    /// Repository-relative file path (forward slashes).
    pub file_path: String,
    /// Line number in the file (1-based).
    pub line_number: usize,
    /// The content of the line that matched.
    pub content: String,
    /// The pattern that matched.
    pub matched_pattern: String,
}

impl StubViolation {
    /// Create a new stub violation.
    pub fn new(
        file_path: impl Into<String>,
        line_number: usize,
        content: impl Into<String>,
        matched_pattern: impl Into<String>,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            line_number,
            content: content.into(),
            matched_pattern: matched_pattern.into(),
        }
    }
}

/// Result of stub validation.
#[derive(Debug, Clone)]
pub struct StubValidationResult {
    /// Whether validation passed (no stubs found).
    pub passed: bool,
    /// List of violations (empty if passed).
    pub violations: Vec<StubViolation>,
}

impl StubValidationResult {
    /// Create a passing result.
    pub fn pass() -> Self {
        Self {
            passed: true,
            violations: Vec::new(),
        }
    }

    /// Create a failing result with violations.
    pub fn fail(violations: Vec<StubViolation>) -> Self {
        Self {
            passed: false,
            violations,
        }
    }

    /// Format the result as a user-friendly error message.
    ///
    /// Output format matches PRD spec:
    /// ```text
    /// Stub patterns found in added lines
    ///
    /// src/player/jump.rs:67  + unimplemented!()
    /// src/player/jump.rs:45  + // TODO: implement cooldown
    /// ```
    pub fn format_error(&self) -> String {
        if self.passed {
            return String::new();
        }

        let mut msg = String::from("Stub patterns found in added lines\n\n");

        for violation in &self.violations {
            msg.push_str(&format!(
                "{}:{}  + {}\n",
                violation.file_path, violation.line_number, violation.content
            ));
        }

        msg
    }
}
