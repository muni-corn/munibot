//! Rate limiting and spend caps.
//!
//! Moved forward from milestone 5 to land with the interface that exposes
//! them: a public web chat is a far larger cost surface than a Discord bot
//! in a handful of guilds - no invite gate, no channel gate, and one
//! signed-in stranger can open unlimited conversations.

mod concurrency;
mod error;
mod limiter;
mod policy;
mod scope;
mod spend_enforcer;
mod spend_error;
mod spend_policy;
mod spend_store;
mod store;

pub use concurrency::ConcurrencyGuard;
pub use error::RateLimitError;
pub use limiter::RateLimiter;
pub use policy::{RateLimitPolicy, ScopePolicies};
pub use scope::Scope;
pub use spend_enforcer::{SpendCapEnforcer, SpendCapStatus};
pub use spend_error::SpendCapError;
pub use spend_policy::{SpendCapPolicies, SpendCapPolicy};
pub use spend_store::{DieselSpendCapStore, SpendCapRow, SpendCapStore};
pub use store::{DieselRateLimitStore, RateLimitStore, RateLimitWindow};
