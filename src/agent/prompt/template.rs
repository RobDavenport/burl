//! Template engine for variable substitution.
//!
//! This module provides a simple template engine that performs `{variable}`
//! substitution in strings. It is used for:
//!
//! - Prompt templates (generating context-aware prompts for agents)
//! - Command templates (substituting task variables into agent commands)
//!
//! # Syntax
//!
//! - `{name}` - Substitutes the value of variable `name`
//! - `{{` - Renders as literal `{`
//! - `}}` - Renders as literal `}`
//!
//! # Error Handling
//!
//! The engine is fail-safe: undefined variables cause an error rather than
//! silent substitution with empty strings. This prevents subtle bugs from
//! typos in variable names.

use std::collections::HashMap;
use std::fmt;

/// Error type for template rendering failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// A variable was referenced but not provided.
    UndefinedVariable {
        /// The name of the undefined variable.
        name: String,
        /// The position in the template where the variable was found.
        position: usize,
    },
    /// A `{` was found without a matching `}`.
    UnmatchedBrace {
        /// The position of the unmatched `{`.
        position: usize,
    },
    /// An empty variable name was found (e.g., `{}`).
    EmptyVariableName {
        /// The position of the empty variable.
        position: usize,
    },
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::UndefinedVariable { name, position } => {
                write!(
                    f,
                    "undefined variable '{}' at position {} in template",
                    name, position
                )
            }
            TemplateError::UnmatchedBrace { position } => {
                write!(f, "unmatched '{{' at position {} in template", position)
            }
            TemplateError::EmptyVariableName { position } => {
                write!(
                    f,
                    "empty variable name '{{}}' at position {} in template",
                    position
                )
            }
        }
    }
}

impl std::error::Error for TemplateError {}

/// Render a template string by substituting variables.
///
/// # Arguments
///
/// * `template` - The template string containing `{variable}` placeholders
/// * `variables` - A map of variable names to their values
///
/// # Returns
///
/// * `Ok(String)` - The rendered string with all variables substituted
/// * `Err(TemplateError)` - If a variable is undefined or syntax is invalid
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use burl::agent::prompt::render_template;
///
/// let mut vars = HashMap::new();
/// vars.insert("name".to_string(), "Alice".to_string());
/// vars.insert("task".to_string(), "coding".to_string());
///
/// let result = render_template("Hello {name}, your task is {task}.", &vars).unwrap();
/// assert_eq!(result, "Hello Alice, your task is coding.");
/// ```
///
/// # Escaping
///
/// Use `{{` to render a literal `{`:
///
/// ```
/// use std::collections::HashMap;
/// use burl::agent::prompt::render_template;
///
/// let vars = HashMap::new();
/// let result = render_template("Use {{var}} for variables", &vars).unwrap();
/// assert_eq!(result, "Use {var} for variables");
/// ```
pub fn render_template(
    template: &str,
    variables: &HashMap<String, String>,
) -> Result<String, TemplateError> {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();

    while let Some((pos, ch)) = chars.next() {
        match ch {
            '{' => {
                // Check for escape sequence {{
                if let Some((_, '{')) = chars.peek() {
                    chars.next(); // consume the second {
                    result.push('{');
                } else {
                    // Parse variable name
                    let start_pos = pos;
                    let mut var_name = String::new();

                    loop {
                        match chars.next() {
                            Some((_, '}')) => break,
                            Some((_, c)) => var_name.push(c),
                            None => {
                                return Err(TemplateError::UnmatchedBrace {
                                    position: start_pos,
                                });
                            }
                        }
                    }

                    // Check for empty variable name
                    if var_name.is_empty() {
                        return Err(TemplateError::EmptyVariableName {
                            position: start_pos,
                        });
                    }

                    // Trim whitespace from variable name for flexibility
                    let var_name = var_name.trim();

                    // Look up the variable
                    match variables.get(var_name) {
                        Some(value) => result.push_str(value),
                        None => {
                            return Err(TemplateError::UndefinedVariable {
                                name: var_name.to_string(),
                                position: start_pos,
                            });
                        }
                    }
                }
            }
            '}' => {
                // Check for escape sequence }}
                if let Some((_, '}')) = chars.peek() {
                    chars.next(); // consume the second }
                    result.push('}');
                } else {
                    // Lone } is just a regular character
                    result.push('}');
                }
            }
            _ => result.push(ch),
        }
    }

    Ok(result)
}

/// Helper to create a variables map from a list of key-value pairs.
#[allow(dead_code)]
pub fn vars<I, K, V>(pairs: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    pairs
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_substitution() {
        let vars = vars([("name", "Alice"), ("greeting", "Hello")]);
        let result = render_template("{greeting}, {name}!", &vars).unwrap();
        assert_eq!(result, "Hello, Alice!");
    }

    #[test]
    fn test_no_variables() {
        let vars = HashMap::new();
        let result = render_template("Just plain text", &vars).unwrap();
        assert_eq!(result, "Just plain text");
    }

    #[test]
    fn test_empty_template() {
        let vars = HashMap::new();
        let result = render_template("", &vars).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_escape_braces() {
        let vars = HashMap::new();
        let result = render_template("Use {{var}} for variables", &vars).unwrap();
        assert_eq!(result, "Use {var} for variables");
    }

    #[test]
    fn test_escape_closing_brace() {
        let vars = HashMap::new();
        let result = render_template("Example: a }} b", &vars).unwrap();
        assert_eq!(result, "Example: a } b");
    }

    #[test]
    fn test_mixed_escapes_and_variables() {
        let vars = vars([("x", "value")]);
        let result = render_template("{{escaped}} and {x}", &vars).unwrap();
        assert_eq!(result, "{escaped} and value");
    }

    #[test]
    fn test_undefined_variable_error() {
        let vars = HashMap::new();
        let result = render_template("Hello {name}", &vars);
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            TemplateError::UndefinedVariable { name, position } => {
                assert_eq!(name, "name");
                assert_eq!(position, 6);
            }
            _ => panic!("unexpected error type: {:?}", err),
        }
    }

    #[test]
    fn test_unmatched_brace_error() {
        let vars = HashMap::new();
        let result = render_template("Hello {name", &vars);
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            TemplateError::UnmatchedBrace { position } => {
                assert_eq!(position, 6);
            }
            _ => panic!("unexpected error type: {:?}", err),
        }
    }

    #[test]
    fn test_empty_variable_name_error() {
        let vars = HashMap::new();
        let result = render_template("Hello {}", &vars);
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            TemplateError::EmptyVariableName { position } => {
                assert_eq!(position, 6);
            }
            _ => panic!("unexpected error type: {:?}", err),
        }
    }

    #[test]
    fn test_whitespace_in_variable_name() {
        let vars = vars([("name", "Alice")]);
        // Whitespace is trimmed
        let result = render_template("Hello { name }!", &vars).unwrap();
        assert_eq!(result, "Hello Alice!");
    }

    #[test]
    fn test_multiple_occurrences() {
        let vars = vars([("x", "X")]);
        let result = render_template("{x}-{x}-{x}", &vars).unwrap();
        assert_eq!(result, "X-X-X");
    }

    #[test]
    fn test_adjacent_variables() {
        let vars = vars([("a", "A"), ("b", "B")]);
        let result = render_template("{a}{b}", &vars).unwrap();
        assert_eq!(result, "AB");
    }

    #[test]
    fn test_lone_closing_brace() {
        let vars = HashMap::new();
        let result = render_template("a } b", &vars).unwrap();
        assert_eq!(result, "a } b");
    }

    #[test]
    fn test_empty_value_substitution() {
        let vars = vars([("empty", "")]);
        let result = render_template("before{empty}after", &vars).unwrap();
        assert_eq!(result, "beforeafter");
    }

    #[test]
    fn test_multiline_template() {
        let vars = vars([("title", "Test Task"), ("objective", "Do something")]);
        let template = "# {title}\n\n## Objective\n{objective}";
        let result = render_template(template, &vars).unwrap();
        assert_eq!(result, "# Test Task\n\n## Objective\nDo something");
    }

    #[test]
    fn test_complex_values() {
        let vars = vars([
            ("code", "fn main() { println!(\"hello\"); }"),
            ("path", "/path/to/file.rs"),
        ]);
        let result = render_template("Code: {code}\nPath: {path}", &vars).unwrap();
        assert_eq!(
            result,
            "Code: fn main() { println!(\"hello\"); }\nPath: /path/to/file.rs"
        );
    }

    #[test]
    fn test_vars_helper() {
        let vars = vars([("a", "1"), ("b", "2")]);
        assert_eq!(vars.get("a"), Some(&"1".to_string()));
        assert_eq!(vars.get("b"), Some(&"2".to_string()));
    }

    #[test]
    fn test_error_display() {
        let err = TemplateError::UndefinedVariable {
            name: "foo".to_string(),
            position: 10,
        };
        assert_eq!(
            err.to_string(),
            "undefined variable 'foo' at position 10 in template"
        );

        let err = TemplateError::UnmatchedBrace { position: 5 };
        assert_eq!(err.to_string(), "unmatched '{' at position 5 in template");

        let err = TemplateError::EmptyVariableName { position: 3 };
        assert_eq!(
            err.to_string(),
            "empty variable name '{}' at position 3 in template"
        );
    }

    #[test]
    fn test_variable_at_start() {
        let vars = vars([("x", "value")]);
        let result = render_template("{x} at start", &vars).unwrap();
        assert_eq!(result, "value at start");
    }

    #[test]
    fn test_variable_at_end() {
        let vars = vars([("x", "value")]);
        let result = render_template("at end {x}", &vars).unwrap();
        assert_eq!(result, "at end value");
    }

    #[test]
    fn test_only_variable() {
        let vars = vars([("x", "value")]);
        let result = render_template("{x}", &vars).unwrap();
        assert_eq!(result, "value");
    }

    #[test]
    fn test_newlines_in_value() {
        let vars = vars([("multi", "line1\nline2\nline3")]);
        let result = render_template("Content:\n{multi}", &vars).unwrap();
        assert_eq!(result, "Content:\nline1\nline2\nline3");
    }

    #[test]
    fn test_braces_in_value() {
        let vars = vars([("code", "if (x > 0) { return x; }")]);
        let result = render_template("Code: {code}", &vars).unwrap();
        assert_eq!(result, "Code: if (x > 0) { return x; }");
    }

    #[test]
    fn test_unicode_in_template_and_values() {
        let vars = vars([("emoji", "🎉"), ("text", "日本語")]);
        let result = render_template("Hello {emoji} {text}!", &vars).unwrap();
        assert_eq!(result, "Hello 🎉 日本語!");
    }
}
