//! The tool system: what munibot's personas can do beyond talking.
//!
//! A tool's authority derives from the **invoking human**, never from the
//! model's request. Every tool is tiered by [`RiskTier`], and every tier above
//! [`RiskTier::Safe`] re-checks the invoker's granted tier at invocation time -
//! a persona misconfigured into a tier the invoker lacks must still be refused
//! at the point of use, not just filtered out of the schema list handed to the
//! model.

pub mod context;
pub mod current_time;
pub mod registry;
pub mod selection;
pub mod tier;
pub mod todo_write;
pub mod tool;

pub use context::{ConversationId, Platform, ToolCtx};
pub use current_time::CurrentTimeTool;
pub use registry::ToolRegistry;
pub use selection::{ToolSelection, ToolSelector};
pub use tier::RiskTier;
pub use todo_write::{TodoItem, TodoStatus, TodoWriteTool};
pub use tool::{Tool, ToolOutcome};
