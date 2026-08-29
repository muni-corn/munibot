//! Forge-agnostic version control types and traits.
//!
//! munibot's autonomous development pipeline (see
//! `docs/plans/ai/milestone-5-autonomous.md`) never speaks a specific
//! forge's own wire format directly. Every forge integration (starting with
//! `munibot_github`) normalizes into the types and implements the traits this
//! crate defines, so the pipeline itself holds only `Arc<dyn IssueSource>`
//! and `Arc<dyn PullRequestTarget>` and never learns a new forge exists.
//!
//! No forge-specific dependency (an API client, a webhook signing scheme)
//! belongs here, ever -- that is exactly what keeps a second forge a matter
//! of writing one more crate against these traits, rather than a rewrite of
//! the pipeline that consumes them.

mod reference;

pub use reference::{Comment, Forge, Issue, IssueRef, IssueState, RepoRef};
