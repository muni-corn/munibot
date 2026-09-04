# GitHub forge verification gaps

What `munibot_github::GitHubForge` (phase 20) actually verified, and what it
didn't - written down so it isn't discovered by surprise during rollout.

**Closed as of commit 242.** The `wiremock`-backed suite this note asked for
is `munibot_github/tests/wiremock_forge.rs`, built on a new
`GitHubForge::from_parts` constructor that takes an already-built
`Octocrab` client (pointed at the mock server's `base_uri`) instead of
minting one from a real GitHub App key, so none of it needs a real
installation. It covers all three paths named below: `create_branch`'s
idempotent reuse (a 422 "reference already exists" response is swallowed,
same as a real repeat delivery would produce) and a real failure response
(a 403, propagated rather than swallowed), and `open_pull_request` end to
end against mocked `POST /repos/{owner}/{repo}/pulls`. `push`'s failure
path is tested differently, and deliberately not against wiremock: it shells
out to a real `git` binary, so its own suite instead points it at a
nonexistent repository on real `github.com` and asserts the resulting
`ForgeError` - proving the actual rejected-push code path rather than a
plausible-looking stand-in for one.

## What's verified

- Token minting and its refresh-before-expiry cache
  (`munibot_github/src/auth.rs`), fully against a mock `TokenMinter` - no
  real network call needed to prove the caching behaviour itself.
- Webhook signature verification (`munibot_github/src/webhook/signature.rs`)
  against a known-good HMAC-SHA256 vector.
- Webhook payload normalization (`munibot_github/src/webhook/normalize.rs`)
  against realistic, hand-written GitHub payload fixtures for every event
  type and action munibot acts on.
- `GitHubForge`'s own error classification
  (`classify_github_error`/`map_error`) and clone url construction
  (`build_clone_url`), both pure functions tested directly.
- `create_branch`, `push`, and `open_pull_request` (commit 242, above) -
  against a real `octocrab` client talking to a `wiremock` server standing
  in for the GitHub REST API, and against real `github.com` for `push`'s
  failure path.

## What's still only verified by inspection

`fetch_issue`, `list_comments`, `post_comment`, and `clone_url_with_token`
still call real `octocrab` methods with no test against a mocked or real
server. This is not a minor gap: `fetch_issue` is what
`PipelineLauncher::launch` calls to resolve the issue a webhook named
(`munibot_gui/src/server/pipeline_launcher.rs:314,380`), and
`post_comment`/`list_comments` are what
`munibot_ai::pipeline::github_interaction` uses for the
`AwaitingUserInput` round trip - a pipeline depends on all three as much
as it depends on the three commit 242 covered. `octocrab::GitHubError` is
`#[non_exhaustive]` with no public constructor, which is why the error
classification test above takes a bare `(StatusCode, &str)` rather than a
real `octocrab::Error` - the mapping logic is proven, but whether
`octocrab` actually raises `Error::GitHub` the way this code assumes was,
until commit 242, unverified for any of `GitHubForge`'s methods. It's
still unverified for these four specifically, and covering them the same
way commit 242 covered `create_branch`/`push`/`open_pull_request` is
unscheduled follow-up work, not something this milestone's plan asks for.
