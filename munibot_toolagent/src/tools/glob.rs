//! The `glob` tool: finds files by pattern, respecting `.gitignore`.

use std::path::PathBuf;

use async_trait::async_trait;
use globset::Glob;
use ignore::WalkBuilder;
use serde::Deserialize;
use serde_json::Value;

use crate::{protocol::ToolResult, server::ToolHandler};

#[derive(Deserialize)]
struct GlobArgs {
    /// A gitignore-style glob pattern, e.g. `"**/*.rs"` or `"src/**"`.
    pattern: String,
}

/// Finds files under the repository root matching a glob pattern.
///
/// Walks with the [`ignore`] crate, which respects `.gitignore` (and
/// `.git/info/exclude`, and a global gitignore) the same way `git` itself
/// would, so generated and vendored files never show up unless the
/// repository's own ignore rules already surface them. Never follows
/// symlinks - a glob is a read-only listing, and there is no reason for it to
/// be able to see outside the repository root at all.
///
/// Results are sorted by modification time, most recent first, so a file the
/// model (or a build it just ran) just touched surfaces before older,
/// unrelated matches.
pub struct GlobHandler {
    root: PathBuf,
}

impl GlobHandler {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ToolHandler for GlobHandler {
    async fn call(&self, input: Value) -> ToolResult {
        let args: GlobArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolResult::Err(format!("couldn't parse arguments :< {error}")),
        };

        let root = self.root.clone();
        let pattern = args.pattern.clone();
        let result = tokio::task::spawn_blocking(move || run_glob(&root, &pattern)).await;

        match result {
            Ok(Ok(matches)) if matches.is_empty() => {
                ToolResult::Ok(format!("no files matched {:?}", args.pattern))
            }
            Ok(Ok(matches)) => ToolResult::Ok(matches.join("\n")),
            Ok(Err(error)) => ToolResult::Err(error),
            Err(error) => ToolResult::Err(format!("glob search panicked :< {error}")),
        }
    }
}

/// Walks `root`, matching every file's path (relative to `root`) against
/// `pattern`, sorted by modification time descending.
///
/// Deliberately does not use [`ignore::overrides`] for the pattern match:
/// an override is designed to *supersede* `.gitignore` (the same mechanism
/// ripgrep's own `-g`/`--glob` flag uses, which intentionally surfaces a
/// gitignored file if it matches), which is the opposite of what a plain
/// glob search wants here. Matching with [`globset`] directly against a
/// walk that has gitignore filtering left in its normal, un-overridden
/// state keeps both behaving independently: the pattern only narrows what a
/// normal walk would already show.
fn run_glob(root: &std::path::Path, pattern: &str) -> Result<Vec<String>, String> {
    let matcher = Glob::new(pattern)
        .map_err(|error| format!("{pattern:?} isn't a valid glob pattern :< {error}"))?
        .compile_matcher();

    let walker = WalkBuilder::new(root)
        .follow_links(false)
        // a sandboxed checkout is always a real git repository in
        // production, but doesn't need to look like one for .gitignore
        // rules to apply here - a plain checked-out directory with a
        // .gitignore file should behave the same way
        .require_git(false)
        .build();

    let mut matches: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            // an unreadable entry (a permissions error, a broken pipe) is
            // skipped rather than failing the whole search over one bad
            // entry among possibly thousands
            Err(error) => {
                tracing::debug!(%error, "skipping an unreadable entry during glob");
                continue;
            }
        };

        let is_file = entry.file_type().is_some_and(|kind| kind.is_file());
        if !is_file {
            continue;
        }

        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        if !matcher.is_match(relative) {
            continue;
        }

        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        matches.push((entry.path().to_path_buf(), modified));
    }

    matches.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));

    Ok(matches
        .into_iter()
        .map(|(path, _)| {
            path.strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
        })
        .collect())
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
                "munibot_toolagent_glob_test_{name}_{}",
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
                // gives each file a distinct, deterministic mtime a moment
                // apart from the others, so sort order is never a coin flip
                // on a fast filesystem where two writes land in the same tick
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Self { root }
        }

        fn handler(&self) -> GlobHandler {
            GlobHandler::new(self.root.clone())
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[tokio::test]
    async fn test_finds_files_matching_a_pattern() {
        let repo = Repo::new("basic", &[
            ("src/main.rs", ""),
            ("src/lib.rs", ""),
            ("README.md", ""),
        ]);

        let outcome = repo.handler().call(json!({"pattern": "**/*.rs"})).await;
        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("src/main.rs"));
                assert!(text.contains("src/lib.rs"));
                assert!(!text.contains("README.md"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_results_are_sorted_most_recently_modified_first() {
        let repo = Repo::new("sort_order", &[
            ("a.rs", ""), // written first, so oldest
            ("b.rs", ""),
            ("c.rs", ""), // written last, so newest
        ]);

        let outcome = repo.handler().call(json!({"pattern": "*.rs"})).await;
        match outcome {
            ToolResult::Ok(text) => {
                let lines: Vec<&str> = text.lines().collect();
                assert_eq!(lines, vec!["c.rs", "b.rs", "a.rs"]);
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_respects_gitignore() {
        let repo = Repo::new("gitignore", &[
            (".gitignore", "ignored.rs\n"),
            ("kept.rs", ""),
            ("ignored.rs", ""),
        ]);

        let outcome = repo.handler().call(json!({"pattern": "*.rs"})).await;
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
        let repo = Repo::new("no_matches", &[("a.txt", "")]);
        let outcome = repo
            .handler()
            .call(json!({"pattern": "**/*.nonexistent_extension"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => assert!(text.contains("no files matched")),
            other => panic!("expected success reporting no matches, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let repo = Repo::new("malformed", &[]);
        let outcome = repo.handler().call(json!({})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }
}
