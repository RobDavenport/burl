//! Prompt generation subsystem for agent execution.
//!
//! This module provides:
//!
//! - **Template**: Variable substitution engine for prompts and commands
//! - **Context**: Task context extraction for template variables
//! - **Generator**: Prompt file generation and writing
//!
//! # Template Syntax
//!
//! Templates use `{variable}` placeholders:
//!
//! ```text
//! # Task: {title}
//!
//! ## Objective
//! {objective}
//!
//! ## Files to modify
//! {affects}
//! ```
//!
//! Use `{{` to escape and render a literal `{`.

mod context;
mod generator;
mod template;

pub use context::TaskContext;
pub use generator::{GeneratedPrompt, generate_and_write_prompt, generate_prompt, write_prompt};
pub use template::{TemplateError, render_template};
