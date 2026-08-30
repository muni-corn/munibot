# GitHub forge verification gaps

What `munibot_github::GitHubForge` (phase 20) actually verified, and what it
didn't - written down so it isn't discovered by surprise during rollout.

**Still accurate as of writing, and now scheduled.** The `wiremock`-backed
suite this note asks for is `docs/plans/ai/milestone-7-projects.md` commit
242, covering exactly the three paths named at the bottom: `create_branch`'s
idempotent reuse, `push`'s rejection path, and `open_pull_request`.

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

## What's still only verified by inspection

`GitHubForge`'s actual trait method bodies (`fetch_issue`, `list_comments`,
`post_comment`, `create_branch`, `push`, `open_pull_request`,
`clone_url_with_token`) call real `octocrab` methods and, for `push`, shell
out to a real `git` binary - none of this has run against a real GitHub App
installation or a mocked HTTP server yet. `octocrab::GitHubError` is
`#[non_exhaustive]` with no public constructor, which is why the error
classification test above takes a bare `(StatusCode, &str)` rather than a
real `octocrab::Error` - the mapping logic is proven, but the "does
`octocrab` actually raise `Error::GitHub` the way this code assumes"
question is not.

**Before this ships against a real repository**: stand up a GitHub App on a
throwaway test organization (or a `wiremock`-backed fake of the GitHub REST
API), and exercise `create_branch`'s idempotent-reuse branch, `push`'s
failure path (a rejected push, not just a successful one), and
`open_pull_request` end to end at least once each.
