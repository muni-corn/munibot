// munibot_config: wasm-safe configuration schema and value types.
//
// this crate describes *what* a configuration provider's settings look
// like (a `ConfigSchema` tree) and *what values* are currently set (a flat
// `ConfigValues` map), without depending on anything server-only (diesel,
// tokio, poise, serenity). that's what lets it be shared between the
// server (where `ConfigurationProvider`s live, in `munibot_core`) and the
// wasm gui client (which renders a schema generically, with no knowledge
// of any particular provider).
//
// see docs/plans/gui-configuration.md for the full design.

pub mod descriptor;
pub mod duration;
pub mod field;
pub mod field_value;
pub mod node;
pub mod path;
pub mod refs;
pub mod scope;
pub mod toggled;
pub mod value;

pub use descriptor::ProviderDescriptor;
pub use duration::SettingDuration;
pub use field::{ChoiceOption, Condition, Field, FieldKind, KeyKind, RefKind};
pub use field_value::{ConfigField, ValueError};
pub use node::{ConfigNode, ConfigSchema};
pub use path::{ConfigPath, ConfigPathError, PathSegment};
pub use refs::{ChannelRef, RoleRef, UserRef};
pub use scope::{ConfigScope, ScopeKind};
pub use toggled::Toggled;
pub use value::{ConfigValue, ConfigValues};
