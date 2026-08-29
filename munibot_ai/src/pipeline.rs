//! The autonomous development pipeline: triage, research, planning, review,
//! implementation, and a pull request, orchestrated over the same harness
//! and personas every other delegable role already uses.
//!
//! See `docs/plans/ai/milestone-5-autonomous.md` for the full design. This
//! module reaches `munibot_vcs::IssueSource` and
//! `munibot_vcs::PullRequestTarget` through that crate directly, never
//! `munibot_github` -- the pipeline is forge-agnostic by construction, the
//! same way the harness itself is provider-agnostic.

mod state;

pub use state::{InteractionRequest, PipelineId, PipelineState, SubtaskId};
