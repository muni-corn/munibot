//! The rig-backed [`Provider`](crate::provider::Provider) implementation.
//!
//! Everything that touches a `rig_core` type lives under this module.
//! [`convert`] holds the pure type conversions; later commits add the adapter
//! that drives them against a real client.

pub mod adapter;
pub mod convert;
pub mod errors;
pub mod resolve;

pub use adapter::RigProvider;
pub use errors::classify_completion_error;
pub use resolve::ProviderResolver;
