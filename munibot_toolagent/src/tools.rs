//! Tool executions available to the rpc server's [`crate::server::Dispatcher`],
//! one module per tool: `read`, `glob`, `write`, `edit`, `grep`, and `bash`.
//!
//! Every tool here resolves any path it touches through
//! [`crate::jail::resolve_in_jail`] before it reaches the filesystem or a
//! shell - never trusting a model-authored path directly.

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
pub mod write;
