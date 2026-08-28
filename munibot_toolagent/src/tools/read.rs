//! The `read` tool: returns a file's contents, line-numbered and bounded.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::{jail::resolve_in_jail, protocol::ToolResult, server::ToolHandler};

/// How many lines `read` returns when the caller doesn't say otherwise.
const DEFAULT_LIMIT: usize = 2000;

/// The longest a single returned line is allowed to be before it gets cut
/// off - a single absurdly long line (a minified bundle, a data dump)
/// should not be able to blow out a turn's context budget on its own.
const MAX_LINE_WIDTH: usize = 2000;

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    /// The 1-indexed line to start from. Defaults to `1`.
    offset: Option<usize>,
    /// How many lines to return. Defaults to [`DEFAULT_LIMIT`].
    limit: Option<usize>,
}

/// Reads a file inside the repository, returning `<line>: <content>`
/// prefixed output so the model can refer back to exact line numbers (in an
/// `edit` call, say) without having to count them itself.
pub struct ReadHandler {
    root: PathBuf,
}

impl ReadHandler {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ToolHandler for ReadHandler {
    async fn call(&self, input: Value) -> ToolResult {
        let args: ReadArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolResult::Err(format!("couldn't parse arguments :< {error}")),
        };

        let resolved = match resolve_in_jail(&self.root, &args.path) {
            Ok(path) => path,
            Err(error) => return ToolResult::Err(format!("{error}")),
        };

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => content,
            Err(error) => {
                return ToolResult::Err(format!("couldn't read {:?} :< {error}", args.path));
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let offset = args.offset.unwrap_or(1).max(1);
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        let start = offset - 1;

        if start >= lines.len() && !lines.is_empty() {
            return ToolResult::Err(format!(
                "{:?} has {} lines total, but offset {offset} is past the end",
                args.path,
                lines.len()
            ));
        }

        let end = start.saturating_add(limit).min(lines.len());
        let formatted = lines[start..end]
            .iter()
            .enumerate()
            .map(|(index, line)| format!("{}: {}", start + index + 1, truncate_line(line)))
            .collect::<Vec<_>>()
            .join("\n");

        ToolResult::Ok(formatted)
    }
}

/// Cuts a line down to [`MAX_LINE_WIDTH`] characters, marking that it was
/// truncated rather than silently dropping the rest with no indication.
fn truncate_line(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_WIDTH {
        return line.to_string();
    }

    let truncated: String = line.chars().take(MAX_LINE_WIDTH).collect();
    format!("{truncated}... (line truncated)")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Repo {
        root: PathBuf,
    }

    impl Repo {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "munibot_toolagent_read_test_{name}_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            for (path, content) in files {
                let full = root.join(path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, content).unwrap();
            }
            Self { root }
        }

        fn handler(&self) -> ReadHandler {
            ReadHandler::new(self.root.clone())
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[tokio::test]
    async fn test_reads_a_whole_small_file_with_line_numbers() {
        let repo = Repo::new("whole_file", &[("src/main.rs", "fn main() {}\n// done")]);
        let outcome = repo.handler().call(json!({"path": "src/main.rs"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                assert_eq!(text, "1: fn main() {}\n2: // done");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_offset_skips_leading_lines() {
        let repo = Repo::new("offset", &[("f.txt", "one\ntwo\nthree")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.txt", "offset": 2}))
            .await;

        match outcome {
            ToolResult::Ok(text) => assert_eq!(text, "2: two\n3: three"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_limit_bounds_how_many_lines_come_back() {
        let repo = Repo::new("limit", &[("f.txt", "one\ntwo\nthree\nfour")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.txt", "limit": 2}))
            .await;

        match outcome {
            ToolResult::Ok(text) => assert_eq!(text, "1: one\n2: two"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_offset_past_the_end_of_the_file_is_a_recoverable_error() {
        let repo = Repo::new("offset_past_end", &[("f.txt", "one\ntwo")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.txt", "offset": 50}))
            .await;

        match outcome {
            ToolResult::Err(message) => assert!(message.contains("past the end")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_line_over_the_width_limit_is_truncated_with_a_marker() {
        let long_line = "x".repeat(MAX_LINE_WIDTH + 100);
        let repo = Repo::new("long_line", &[("f.txt", &long_line)]);
        let outcome = repo.handler().call(json!({"path": "f.txt"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("(line truncated)"));
                assert!(text.len() < long_line.len());
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_missing_file_is_a_recoverable_error() {
        let repo = Repo::new("missing_file", &[]);
        let outcome = repo
            .handler()
            .call(json!({"path": "does_not_exist.rs"}))
            .await;

        assert!(matches!(outcome, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn test_a_path_escaping_the_jail_is_a_recoverable_error() {
        let repo = Repo::new("escape_attempt", &[]);
        let outcome = repo
            .handler()
            .call(json!({"path": "../../../../etc/passwd"}))
            .await;

        match outcome {
            ToolResult::Err(message) => assert!(message.contains("outside the repository root")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let repo = Repo::new("malformed", &[]);
        let outcome = repo.handler().call(json!({})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }
}
