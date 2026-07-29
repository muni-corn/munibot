//! The agent loop: model to tools to handoff, with budgets and events.
//!
//! This is the one place that drives a [`crate::provider::Provider`] and a
//! [`crate::tools::ToolRegistry`] together. Everything above it - personas,
//! platform adapters, the eventual pipeline - talks to the harness and never to
//! a provider or a tool directly.

pub mod budget;
pub mod event;
pub mod turn;

pub use budget::{Budget, BudgetTracker};
pub use event::HarnessEvent;
pub use turn::{HandoffSchema, TurnOutcome, TurnRequest};
