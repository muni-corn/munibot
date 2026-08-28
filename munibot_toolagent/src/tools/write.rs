//! The `write` tool: creates or overwrites a file inside the repository.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::{jail::resolve_in_jail, protocol::ToolResult, server::ToolHandler};

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

/// Writes `content` to a file inside the repository, creating parent
/// directories as needed and overwriting whatever was there.
///
/// Whether the session has already read a file it's about to overwrite is a
/// host-side policy (see `docs/plans/ai/milestone-4-sandbox.md` commit 145) -
/// this tool has no session state of its own to check that against, and
/// simply does what it's asked.
pub struct WriteHandler {
    root: PathBuf,
}

impl WriteHandler {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ToolHandler for WriteHandler {
    async fn call(&self, input: Value) -> ToolResult {
        let args: WriteArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolResult::Err(format!("couldn't parse arguments :< {error}")),
        };

        let resolved = match resolve_in_jail(&self.root, &args.path) {
            Ok(path) => path,
            Err(error) => return ToolResult::Err(format!("{error}")),
        };

        if let Some(parent) = resolved.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            return ToolResult::Err(format!(
                "couldn't create the directory for {:?} :< {error}",
                args.path
            ));
        }

        let bytes = args.content.len();
        match tokio::fs::write(&resolved, &args.content).await {
            Ok(()) => ToolResult::Ok(format!("wrote {bytes} bytes to {:?}", args.path)),
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
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "munibot_toolagent_write_test_{name}_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn handler(&self) -> WriteHandler {
            WriteHandler::new(self.root.clone())
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[tokio::test]
    async fn test_writes_a_new_file() {
        let repo = Repo::new("new_file");
        let outcome = repo
            .handler()
            .call(json!({"path": "src/main.rs", "content": "fn main() {}"}))
            .await;

        assert!(matches!(outcome, ToolResult::Ok(_)), "got {outcome:?}");
        assert_eq!(
            std::fs::read_to_string(repo.root.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
    }

    #[tokio::test]
    async fn test_creates_parent_directories_that_do_not_exist_yet() {
        let repo = Repo::new("nested_dirs");
        let outcome = repo
            .handler()
            .call(json!({"path": "a/b/c/deep.rs", "content": "// hi"}))
            .await;

        assert!(matches!(outcome, ToolResult::Ok(_)), "got {outcome:?}");
        assert!(repo.root.join("a/b/c/deep.rs").is_file());
    }

    #[tokio::test]
    async fn test_overwrites_an_existing_file() {
        let repo = Repo::new("overwrite");
        std::fs::write(repo.root.join("f.txt"), "old content").unwrap();

        let outcome = repo
            .handler()
            .call(json!({"path": "f.txt", "content": "new content"}))
            .await;

        assert!(matches!(outcome, ToolResult::Ok(_)), "got {outcome:?}");
        assert_eq!(
            std::fs::read_to_string(repo.root.join("f.txt")).unwrap(),
            "new content"
        );
    }

    #[tokio::test]
    async fn test_reports_the_byte_count_written() {
        let repo = Repo::new("byte_count");
        let outcome = repo
            .handler()
            .call(json!({"path": "f.txt", "content": "hello"}))
            .await;

        match outcome {
            ToolResult::Ok(message) => assert!(message.contains('5')),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_path_escaping_the_jail_is_a_recoverable_error() {
        let repo = Repo::new("escape_attempt");
        let outcome = repo
            .handler()
            .call(json!({"path": "../../../../etc/passwd", "content": "pwned"}))
            .await;

        match outcome {
            ToolResult::Err(message) => assert!(message.contains("outside the repository root")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
        assert!(!std::path::Path::new("/etc/passwd_pwned").exists());
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let repo = Repo::new("malformed");
        let outcome = repo.handler().call(json!({"path": "f.txt"})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }
}
