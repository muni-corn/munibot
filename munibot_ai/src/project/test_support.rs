//! Test-only fixtures shared by `ai::project`'s own tests and
//! `ai::service`'s tests of project-bound conversations: a throwaway
//! local git upstream [`ProjectManager::register`] can actually clone,
//! and a store-backed manager rooted at a scratch directory, with real
//! git plumbing and zero network involved either way.

#![cfg(test)]

use std::{path::PathBuf, sync::Arc};

use munibot_vcs::RepoRef;

use crate::{
    project::{InMemoryProjectStore, ProjectManager, ProjectsSettings, RegisterProjectSpec},
    sandbox::config::SandboxNetworkConfig,
};

/// A local bare repository with one commit on `main`, real git
/// plumbing, zero network.
pub(crate) struct TestUpstream {
    bare_path: PathBuf,
}

impl TestUpstream {
    pub(crate) async fn new(files: &[(&str, &str)]) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let bare_path = std::env::temp_dir().join(format!("munibot_ai_test_upstream_{unique}"));
        let work_dir = std::env::temp_dir().join(format!("munibot_ai_test_work_{unique}"));
        std::fs::create_dir_all(&work_dir).unwrap();

        run(&[
            "init",
            "--bare",
            "--initial-branch",
            "main",
            &bare_path.display().to_string(),
        ])
        .await;

        let work_dir_str = work_dir.display().to_string();
        run(&["-C", &work_dir_str, "init", "--initial-branch", "main"]).await;
        run(&[
            "-C",
            &work_dir_str,
            "config",
            "user.email",
            "muni@example.com",
        ])
        .await;
        run(&["-C", &work_dir_str, "config", "user.name", "muni"]).await;
        run(&["-C", &work_dir_str, "config", "commit.gpgsign", "false"]).await;
        for (name, content) in files {
            let path = work_dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        run(&["-C", &work_dir_str, "add", "."]).await;
        run(&["-C", &work_dir_str, "commit", "-m", "initial commit"]).await;
        run(&[
            "-C",
            &work_dir_str,
            "push",
            &bare_path.display().to_string(),
            "main",
        ])
        .await;
        std::fs::remove_dir_all(&work_dir).ok();

        Self { bare_path }
    }

    pub(crate) fn url(&self) -> String {
        format!("file://{}", self.bare_path.display())
    }
}

impl Drop for TestUpstream {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.bare_path).ok();
    }
}

async fn run(args: &[&str]) {
    let status = tokio::process::Command::new("git")
        .args(args)
        .status()
        .await
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// A [`ProjectManager`] backed by an in-memory store, rooted at `root`.
pub(crate) fn manager_with_root(root: PathBuf) -> (ProjectManager, Arc<InMemoryProjectStore>) {
    let store = Arc::new(InMemoryProjectStore::new());
    let settings = ProjectsSettings {
        root,
        ..ProjectsSettings::default()
    };
    (ProjectManager::new(store.clone(), settings), store)
}

/// A minimal [`RegisterProjectSpec`] cloning `upstream`, on the `main`
/// branch, with no network access and no forge token.
pub(crate) fn minimal_spec(
    upstream: &TestUpstream,
    munibot_user_id: i64,
    slug: &str,
) -> RegisterProjectSpec {
    RegisterProjectSpec {
        munibot_user_id,
        slug: slug.to_string(),
        display_name: slug.to_string(),
        repo: RepoRef::new(munibot_vcs::Forge::GitHub, "muni-corn", slug),
        clone_url: upstream.url(),
        default_branch: "main".to_string(),
        network_policy: SandboxNetworkConfig::None,
        trigger: None,
        token: None,
        installation_id: None,
    }
}
