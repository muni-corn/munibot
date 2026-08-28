//! The `grep` tool: regex search across the repository, capped and
//! optionally narrowed to files matching an include glob.

use std::path::PathBuf;

use async_trait::async_trait;
use globset::Glob;
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, sinks::UTF8};
use ignore::WalkBuilder;
use serde::Deserialize;
use serde_json::Value;

use crate::{protocol::ToolResult, server::ToolHandler};

/// The most matches `grep` will ever return, across every file searched -
/// a search over an unfamiliar repository can otherwise produce far more
/// output than a turn's context budget can hold.
const MAX_MATCHES: usize = 200;

#[derive(Deserialize)]
struct GrepArgs {
    /// A regular expression, in the same syntax the [`regex`] crate accepts.
    pattern: String,
    /// Only search files whose root-relative path matches this glob, e.g.
    /// `"**/*.rs"`. Searches every file when omitted.
    include: Option<String>,
}

/// One matching line, with enough context to act on without re-reading the
/// whole file.
struct Match {
    path: String,
    line_number: u64,
    line: String,
}

/// Searches every file under the repository root for a regex, over the same
/// [`grep-searcher`]/[`grep-regex`] stack ripgrep itself is built from.
///
/// Walks with the [`ignore`] crate, respecting `.gitignore` the same way
/// [`super::glob::GlobHandler`] does, and for the same reason narrows files
/// with [`globset`] directly rather than [`ignore::overrides`] when
/// `include` is given - an override supersedes `.gitignore`, which a search
/// filter should not.
pub struct GrepHandler {
    root: PathBuf,
}

impl GrepHandler {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ToolHandler for GrepHandler {
    async fn call(&self, input: Value) -> ToolResult {
        let args: GrepArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolResult::Err(format!("couldn't parse arguments :< {error}")),
        };

        let root = self.root.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_grep(&root, &args.pattern, args.include.as_deref())
        })
        .await;

        match result {
            Ok(Ok(matches)) if matches.is_empty() => ToolResult::Ok("no matches found".to_string()),
            Ok(Ok(matches)) => {
                let truncated = matches.len() >= MAX_MATCHES;
                let mut formatted = matches
                    .iter()
                    .map(|m| format!("{}:{}: {}", m.path, m.line_number, m.line))
                    .collect::<Vec<_>>()
                    .join("\n");
                if truncated {
                    formatted.push_str(&format!(
                        "\n... (stopped after {MAX_MATCHES} matches; narrow the pattern or use \
                         include to see more)"
                    ));
                }
                ToolResult::Ok(formatted)
            }
            Ok(Err(error)) => ToolResult::Err(error),
            Err(error) => ToolResult::Err(format!("grep search panicked :< {error}")),
        }
    }
}

fn run_grep(
    root: &std::path::Path,
    pattern: &str,
    include: Option<&str>,
) -> Result<Vec<Match>, String> {
    let matcher = RegexMatcher::new(pattern)
        .map_err(|error| format!("{pattern:?} isn't a valid regular expression :< {error}"))?;

    let include_matcher = match include {
        Some(glob) => Some(
            Glob::new(glob)
                .map_err(|error| format!("{glob:?} isn't a valid glob pattern :< {error}"))?
                .compile_matcher(),
        ),
        None => None,
    };

    let walker = WalkBuilder::new(root)
        .follow_links(false)
        .require_git(false)
        .build();

    let mut matches = Vec::new();

    'files: for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(%error, "skipping an unreadable entry during grep");
                continue;
            }
        };

        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }

        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        if let Some(include_matcher) = &include_matcher
            && !include_matcher.is_match(relative)
        {
            continue;
        }

        let relative_display = relative.display().to_string();
        let mut searcher = Searcher::new();
        let search_result = searcher.search_path(
            &matcher,
            entry.path(),
            UTF8(|line_number, line| {
                matches.push(Match {
                    path: relative_display.clone(),
                    line_number,
                    line: line.trim_end_matches(['\n', '\r']).to_string(),
                });
                // returning Ok(false) stops the searcher for *this* file as
                // soon as the global cap is hit, rather than continuing to
                // search a file whose results will just be discarded
                Ok(matches.len() < MAX_MATCHES)
            }),
        );

        if let Err(error) = search_result {
            tracing::debug!(%error, path = %relative_display, "skipping a file that couldn't be searched");
        }

        if matches.len() >= MAX_MATCHES {
            break 'files;
        }
    }

    Ok(matches)
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
                "munibot_toolagent_grep_test_{name}_{}",
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

        fn handler(&self) -> GrepHandler {
            GrepHandler::new(self.root.clone())
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[tokio::test]
    async fn test_finds_a_matching_line_with_its_number() {
        let repo = Repo::new("basic", &[("src/main.rs", "fn main() {\n    todo!()\n}")]);
        let outcome = repo.handler().call(json!({"pattern": "todo!"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("src/main.rs:2:"), "got {text:?}");
                assert!(text.contains("todo!()"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_searches_across_multiple_files() {
        let repo = Repo::new("multi_file", &[
            ("a.rs", "struct Needle;"),
            ("b.rs", "// nothing here"),
            ("c.rs", "fn uses() -> Needle { Needle }"),
        ]);
        let outcome = repo.handler().call(json!({"pattern": "Needle"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("a.rs:1:"));
                assert!(text.contains("c.rs:1:"));
                assert!(!text.contains("b.rs"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_supports_regular_expressions() {
        let repo = Repo::new("regex", &[("f.rs", "let x1 = 1;\nlet y2 = 2;\nlet z = 3;")]);
        let outcome = repo.handler().call(json!({"pattern": r"[a-z]\d"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("f.rs:1:"));
                assert!(text.contains("f.rs:2:"));
                assert!(!text.contains("f.rs:3:"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_include_narrows_which_files_are_searched() {
        let repo = Repo::new("include_filter", &[
            ("src/main.rs", "target_word"),
            ("README.md", "target_word"),
        ]);
        let outcome = repo
            .handler()
            .call(json!({"pattern": "target_word", "include": "**/*.rs"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("main.rs"));
                assert!(!text.contains("README.md"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_respects_gitignore() {
        let repo = Repo::new("gitignore", &[
            (".gitignore", "ignored.rs\n"),
            ("kept.rs", "needle"),
            ("ignored.rs", "needle"),
        ]);
        let outcome = repo.handler().call(json!({"pattern": "needle"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("kept.rs"));
                assert!(!text.contains("ignored.rs"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_no_matches_is_a_plain_message_not_an_error() {
        let repo = Repo::new("no_matches", &[("a.rs", "nothing interesting")]);
        let outcome = repo
            .handler()
            .call(json!({"pattern": "asdkjfhaslkdjfh"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => assert!(text.contains("no matches")),
            other => panic!("expected success reporting no matches, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_an_invalid_regex_is_a_recoverable_error() {
        let repo = Repo::new("bad_regex", &[("a.rs", "x")]);
        let outcome = repo.handler().call(json!({"pattern": "("})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn test_matches_are_capped_at_the_maximum() {
        let content = "needle\n".repeat(MAX_MATCHES + 50);
        let repo = Repo::new("capped", &[("big.rs", &content)]);
        let outcome = repo.handler().call(json!({"pattern": "needle"})).await;

        match outcome {
            ToolResult::Ok(text) => {
                let match_lines = text.lines().filter(|l| l.contains("big.rs:")).count();
                assert_eq!(match_lines, MAX_MATCHES);
                assert!(text.contains("stopped after"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let repo = Repo::new("malformed", &[]);
        let outcome = repo.handler().call(json!({})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }
}
