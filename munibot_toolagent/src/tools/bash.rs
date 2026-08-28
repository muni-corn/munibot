//! The `bash` tool: runs one command through a shell, with a timeout and
//! separate stdout/stderr capture.

use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::{io::AsyncReadExt, process::Command};

use crate::{jail::resolve_in_jail, protocol::ToolResult, server::ToolHandler};

/// How long a call runs before it's killed, when the caller doesn't say
/// otherwise.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// The longest timeout this tool will ever honour, regardless of what's
/// asked for. The container's own wall-clock ceiling (`SandboxConfig`,
/// commit 141) is a second, independent bound on the whole sandbox's
/// lifetime - this one exists so a single call can't quietly consume all of
/// it by itself.
const MAX_TIMEOUT_SECS: u64 = 600;

/// How much of each stream is shown inline before output is paged out to a
/// file instead.
const MAX_VISIBLE_BYTES: usize = 4_000;

/// Where paged output gets written, relative to the repository root - a
/// hidden directory rather than `/tmp`, so it stays inside the jail and is
/// readable back through the ordinary `read` tool.
const PAGE_DIRECTORY: &str = ".agent-pages";

#[derive(Deserialize)]
struct BashArgs {
    command: String,
    /// Relative to the repository root. Defaults to the root itself.
    working_directory: Option<String>,
    /// Defaults to [`DEFAULT_TIMEOUT_SECS`], clamped to
    /// [`MAX_TIMEOUT_SECS`].
    timeout_secs: Option<u64>,
}

/// Runs `command` through `sh -c`, capturing stdout and stderr separately.
///
/// Marked [`Tool::is_serial`](crate::server::ToolHandler) at the tier
/// registered on the host side (`ai::sandbox`, commit 145) rather than
/// here - this handler itself has no shared state across calls, only the
/// host's own dispatch needs to know a shell session shouldn't race itself.
pub struct BashHandler {
    root: PathBuf,
}

impl BashHandler {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ToolHandler for BashHandler {
    async fn call(&self, input: Value) -> ToolResult {
        let args: BashArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolResult::Err(format!("couldn't parse arguments :< {error}")),
        };

        let cwd = match &args.working_directory {
            Some(dir) => match resolve_in_jail(&self.root, dir) {
                Ok(path) => path,
                Err(error) => return ToolResult::Err(format!("{error}")),
            },
            None => match std::fs::canonicalize(&self.root) {
                Ok(path) => path,
                Err(error) => {
                    return ToolResult::Err(format!(
                        "couldn't resolve the repository root :< {error}"
                    ));
                }
            },
        };

        if !cwd.is_dir() {
            return ToolResult::Err(format!(
                "{:?} isn't a directory",
                args.working_directory.as_deref().unwrap_or(".")
            ));
        }

        let timeout = Duration::from_secs(
            args.timeout_secs
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .clamp(1, MAX_TIMEOUT_SECS),
        );

        run_command(&args.command, &cwd, timeout, &self.root).await
    }
}

/// Spawns the shell, races it against `timeout`, and formats whatever
/// finished first.
async fn run_command(
    command: &str,
    cwd: &std::path::Path,
    timeout: Duration,
    root: &std::path::Path,
) -> ToolResult {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return ToolResult::Err(format!("couldn't start a shell :< {error}")),
    };

    let mut stdout_pipe = child.stdout.take().expect("stdout was piped above");
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped above");

    let collect = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (stdout_result, stderr_result, status_result) = tokio::join!(
            stdout_pipe.read_to_end(&mut stdout),
            stderr_pipe.read_to_end(&mut stderr),
            child.wait(),
        );
        (stdout_result, stderr_result, status_result, stdout, stderr)
    };

    match tokio::time::timeout(timeout, collect).await {
        Ok((stdout_result, stderr_result, status_result, stdout, stderr)) => {
            if let Err(error) = stdout_result.as_ref().and(stderr_result.as_ref()) {
                return ToolResult::Err(format!("couldn't read the command's output :< {error}"));
            }
            let status = match status_result {
                Ok(status) => status,
                Err(error) => {
                    return ToolResult::Err(format!(
                        "couldn't wait for the command to finish :< {error}"
                    ));
                }
            };

            ToolResult::Ok(format_output(status.code(), &stdout, &stderr, root).await)
        }
        Err(_elapsed) => {
            // best-effort: the process is already unreachable from this
            // point on either way, so a failure to kill or reap it is
            // logged rather than turning a clear timeout into a confusing
            // secondary error
            let _ = child.start_kill();
            let _ = child.wait().await;
            ToolResult::Err(format!(
                "timed out after {}s :< {command:?}",
                timeout.as_secs()
            ))
        }
    }
}

/// Formats a finished command's result, paging stdout/stderr out to a file
/// under `root` when either stream is too big to show inline.
async fn format_output(
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
    root: &std::path::Path,
) -> String {
    let exit_line = match exit_code {
        Some(code) => format!("exit code: {code}"),
        // only possible if the process was killed by a signal, which the
        // timeout path already reports separately - kept here too in case
        // something else in the environment signals it first
        None => "exit code: (terminated by signal)".to_string(),
    };

    if stdout.len() <= MAX_VISIBLE_BYTES && stderr.len() <= MAX_VISIBLE_BYTES {
        return format!(
            "{exit_line}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(stdout),
            String::from_utf8_lossy(stderr)
        );
    }

    let page_path = match write_page(root, &exit_line, stdout, stderr).await {
        Ok(relative_path) => format!(
            "\nfull output written to {relative_path:?} :< read it with the read tool for the \
             complete, untruncated output"
        ),
        Err(error) => format!("\n(couldn't page the full output to disk :< {error})"),
    };

    format!(
        "{exit_line}\noutput was {} bytes (stdout: {}, stderr: {}), too large to show in \
         full.{page_path}\n\nstdout (first {MAX_VISIBLE_BYTES} bytes):\n{}\n\nstderr (first \
         {MAX_VISIBLE_BYTES} bytes):\n{}",
        stdout.len() + stderr.len(),
        stdout.len(),
        stderr.len(),
        truncate_bytes(stdout),
        truncate_bytes(stderr),
    )
}

fn truncate_bytes(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    if bytes.len() <= MAX_VISIBLE_BYTES {
        return String::from_utf8_lossy(bytes);
    }
    String::from_utf8_lossy(&bytes[..MAX_VISIBLE_BYTES])
        .into_owned()
        .into()
}

/// Writes the complete, untruncated output to a fresh file under
/// [`PAGE_DIRECTORY`], returning its path relative to `root`.
async fn write_page(
    root: &std::path::Path,
    exit_line: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<String, std::io::Error> {
    let page_dir = root.join(PAGE_DIRECTORY);
    tokio::fs::create_dir_all(&page_dir).await?;

    let filename = format!(
        "bash-{}.log",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let full_path = page_dir.join(&filename);

    let mut contents = format!("{exit_line}\n\n=== stdout ===\n").into_bytes();
    contents.extend_from_slice(stdout);
    contents.extend_from_slice(b"\n\n=== stderr ===\n");
    contents.extend_from_slice(stderr);

    tokio::fs::write(&full_path, contents).await?;

    Ok(format!("{PAGE_DIRECTORY}/{filename}"))
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
                "munibot_toolagent_bash_test_{name}_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn handler(&self) -> BashHandler {
            BashHandler::new(self.root.clone())
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[tokio::test]
    async fn test_runs_a_simple_command_and_captures_stdout() {
        let repo = Repo::new("simple");
        let outcome = repo
            .handler()
            .call(json!({"command": "echo hello sandbox"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("exit code: 0"));
                assert!(text.contains("hello sandbox"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_captures_stderr_separately_from_stdout() {
        let repo = Repo::new("stderr");
        let outcome = repo
            .handler()
            .call(json!({"command": "echo to_stdout; echo to_stderr >&2"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => {
                let stdout_section = text.split("stderr:").next().unwrap();
                let stderr_section = text.split("stderr:").nth(1).unwrap();
                assert!(stdout_section.contains("to_stdout"));
                assert!(!stdout_section.contains("to_stderr"));
                assert!(stderr_section.contains("to_stderr"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_reports_a_nonzero_exit_code() {
        let repo = Repo::new("nonzero_exit");
        let outcome = repo.handler().call(json!({"command": "exit 7"})).await;

        match outcome {
            ToolResult::Ok(text) => assert!(text.contains("exit code: 7")),
            other => panic!("expected success reporting the exit code, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_runs_in_the_given_working_directory() {
        let repo = Repo::new("working_directory");
        std::fs::create_dir_all(repo.root.join("subdir")).unwrap();
        std::fs::write(repo.root.join("subdir/marker.txt"), "here").unwrap();

        let outcome = repo
            .handler()
            .call(json!({"command": "cat marker.txt", "working_directory": "subdir"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => assert!(text.contains("here")),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_working_directory_escaping_the_jail_is_a_recoverable_error() {
        let repo = Repo::new("escape_attempt");
        let outcome = repo
            .handler()
            .call(json!({"command": "ls", "working_directory": "../../../../etc"}))
            .await;

        match outcome {
            ToolResult::Err(message) => assert!(message.contains("outside the repository root")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_nonexistent_working_directory_is_a_recoverable_error() {
        let repo = Repo::new("missing_dir");
        let outcome = repo
            .handler()
            .call(json!({"command": "ls", "working_directory": "does_not_exist"}))
            .await;

        assert!(matches!(outcome, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn test_a_slow_command_is_killed_after_its_timeout() {
        let repo = Repo::new("timeout");
        let started = std::time::Instant::now();
        let outcome = repo
            .handler()
            .call(json!({"command": "sleep 30", "timeout_secs": 1}))
            .await;

        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        match outcome {
            ToolResult::Err(message) => assert!(message.contains("timed out")),
            other => panic!("expected a timeout error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_large_output_is_paged_to_a_readable_file() {
        let repo = Repo::new("paged_output");
        let outcome = repo
            .handler()
            .call(json!({"command": "yes line | head -c 20000"}))
            .await;

        match outcome {
            ToolResult::Ok(text) => {
                assert!(text.contains("too large to show in full"));
                assert!(text.contains(".agent-pages/"));

                let page_dir = repo.root.join(".agent-pages");
                let entries: Vec<_> = std::fs::read_dir(&page_dir).unwrap().collect();
                assert_eq!(
                    entries.len(),
                    1,
                    "should have written exactly one page file"
                );

                let paged_content =
                    std::fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
                assert!(
                    paged_content.len() > 20_000,
                    "the page file should hold the full, untruncated output"
                );
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_small_output_is_never_paged() {
        let repo = Repo::new("small_output_not_paged");
        let outcome = repo.handler().call(json!({"command": "echo tiny"})).await;

        assert!(matches!(outcome, ToolResult::Ok(_)));
        assert!(!repo.root.join(".agent-pages").exists());
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let repo = Repo::new("malformed");
        let outcome = repo.handler().call(json!({})).await;
        assert!(matches!(outcome, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn test_timeout_is_clamped_to_the_maximum() {
        // asking for an absurd timeout must not actually wait that long -
        // this command finishes instantly regardless, so this mostly
        // documents the clamp exists rather than proving the wait itself,
        // but it does prove a huge value is accepted rather than rejected
        let repo = Repo::new("timeout_clamp");
        let outcome = repo
            .handler()
            .call(json!({"command": "echo ok", "timeout_secs": 999_999}))
            .await;
        assert!(matches!(outcome, ToolResult::Ok(_)));
    }
}
