//! The `edit` tool: exact string replacement inside a file.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::{jail::resolve_in_jail, protocol::ToolResult, server::ToolHandler};

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    /// Replaces every occurrence rather than just one. Defaults to `false`.
    replace_all: Option<bool>,
}

/// Replaces an exact string inside a file, erroring rather than guessing
/// when the target is absent or ambiguous.
///
/// Ambiguity is always an error unless `replace_all` says otherwise - a
/// silent wrong-match edit (replacing the *first* of several matches when
/// the model meant a different one) is far worse than a failed call the
/// model can recover from by supplying more surrounding context.
pub struct EditHandler {
    root: PathBuf,
}

impl EditHandler {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ToolHandler for EditHandler {
    async fn call(&self, input: Value) -> ToolResult {
        let args: EditArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolResult::Err(format!("couldn't parse arguments :< {error}")),
        };

        if args.old_string.is_empty() {
            return ToolResult::Err("old_string must not be empty :<".to_string());
        }

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

        let occurrences = content.matches(args.old_string.as_str()).count();
        let replace_all = args.replace_all.unwrap_or(false);

        if occurrences == 0 {
            return ToolResult::Err(format!(
                "old_string wasn't found in {:?} :< nothing was changed",
                args.path
            ));
        }
        if occurrences > 1 && !replace_all {
            return ToolResult::Err(format!(
                "old_string appears {occurrences} times in {:?} :< provide more surrounding \
                 context to make it unique, or pass replace_all to replace every occurrence",
                args.path
            ));
        }

        let updated = if replace_all {
            content.replace(args.old_string.as_str(), &args.new_string)
        } else {
            content.replacen(args.old_string.as_str(), &args.new_string, 1)
        };

        match tokio::fs::write(&resolved, updated).await {
            Ok(()) => ToolResult::Ok(format!(
                "replaced {occurrences} occurrence{} in {:?}",
                if occurrences == 1 { "" } else { "s" },
                args.path
            )),
            Err(error) => ToolResult::Err(format!("couldn't write {:?} :< {error}", args.path)),
        }
    }
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
                "munibot_toolagent_edit_test_{name}_{}",
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

        fn handler(&self) -> EditHandler {
            EditHandler::new(self.root.clone())
        }

        fn read(&self, path: &str) -> String {
            std::fs::read_to_string(self.root.join(path)).unwrap()
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[tokio::test]
    async fn test_replaces_a_unique_occurrence() {
        let repo = Repo::new("unique", &[("f.rs", "fn old_name() {}")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.rs", "old_string": "old_name", "new_string": "new_name"}))
            .await;

        assert!(matches!(outcome, ToolResult::Ok(_)), "got {outcome:?}");
        assert_eq!(repo.read("f.rs"), "fn new_name() {}");
    }

    #[tokio::test]
    async fn test_missing_target_string_is_a_recoverable_error_and_changes_nothing() {
        let repo = Repo::new("missing", &[("f.rs", "fn keep_me() {}")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.rs", "old_string": "does_not_exist", "new_string": "x"}))
            .await;

        match outcome {
            ToolResult::Err(message) => assert!(message.contains("wasn't found")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
        assert_eq!(repo.read("f.rs"), "fn keep_me() {}");
    }

    #[tokio::test]
    async fn test_ambiguous_target_is_a_recoverable_error_and_changes_nothing_without_replace_all()
    {
        let repo = Repo::new("ambiguous", &[("f.rs", "foo();\nfoo();\nfoo();")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.rs", "old_string": "foo()", "new_string": "bar()"}))
            .await;

        match outcome {
            ToolResult::Err(message) => {
                assert!(message.contains('3'), "should name the count: {message:?}");
            }
            other => panic!("expected a recoverable error, got {other:?}"),
        }
        assert_eq!(repo.read("f.rs"), "foo();\nfoo();\nfoo();");
    }

    #[tokio::test]
    async fn test_replace_all_replaces_every_occurrence() {
        let repo = Repo::new("replace_all", &[("f.rs", "foo();\nfoo();\nfoo();")]);
        let outcome = repo
            .handler()
            .call(json!({
                "path": "f.rs",
                "old_string": "foo()",
                "new_string": "bar()",
                "replace_all": true,
            }))
            .await;

        assert!(matches!(outcome, ToolResult::Ok(_)), "got {outcome:?}");
        assert_eq!(repo.read("f.rs"), "bar();\nbar();\nbar();");
    }

    #[tokio::test]
    async fn test_replace_all_with_a_single_occurrence_still_succeeds() {
        let repo = Repo::new("replace_all_single", &[("f.rs", "only once")]);
        let outcome = repo
            .handler()
            .call(json!({
                "path": "f.rs",
                "old_string": "once",
                "new_string": "twice",
                "replace_all": true,
            }))
            .await;

        assert!(matches!(outcome, ToolResult::Ok(_)), "got {outcome:?}");
        assert_eq!(repo.read("f.rs"), "only twice");
    }

    #[tokio::test]
    async fn test_an_empty_old_string_is_a_recoverable_error() {
        let repo = Repo::new("empty_old_string", &[("f.rs", "anything")]);
        let outcome = repo
            .handler()
            .call(json!({"path": "f.rs", "old_string": "", "new_string": "x"}))
            .await;

        assert!(matches!(outcome, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn test_a_missing_file_is_a_recoverable_error() {
        let repo = Repo::new("missing_file", &[]);
        let outcome = repo
            .handler()
            .call(json!({"path": "ghost.rs", "old_string": "a", "new_string": "b"}))
            .await;

        assert!(matches!(outcome, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn test_a_path_escaping_the_jail_is_a_recoverable_error() {
        let repo = Repo::new("escape_attempt", &[]);
        let outcome = repo
            .handler()
            .call(json!({
                "path": "../../../../etc/passwd",
                "old_string": "root",
                "new_string": "pwned",
            }))
            .await;

        match outcome {
            ToolResult::Err(message) => assert!(message.contains("outside the repository root")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let repo = Repo::new("malformed", &[]);
        let outcome = repo.handler().call(json!({"path": "f.rs"})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }
}
