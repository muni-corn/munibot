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

pub mod choice;
pub mod configurable;
pub mod constraint;
pub mod descriptor;
pub mod duration;
pub mod field;
pub mod field_value;
pub mod flatten;
pub mod key;
pub mod node;
pub mod patch;
pub mod path;
pub mod refs;
pub mod scope;
pub mod toggled;
pub mod value;

pub use choice::{ConfigChoice, choice_field_kind, choice_from_value, choice_to_value};
pub use configurable::Configurable;
pub use constraint::{validate_numeric_bounds, validate_text_length};
pub use descriptor::ProviderDescriptor;
pub use duration::SettingDuration;
pub use field::{ChoiceOption, Condition, Field, FieldKind, KeyKind, RefKind};
pub use field_value::{ConfigField, ValueError};
pub use flatten::{flatten, unflatten};
pub use key::ConfigKey;
// re-exports the derive macros under the same names as the traits they
// implement (`Configurable`, `ConfigChoice`), the same way serde re-exports
// serde_derive. This is not a name collision: a trait and a derive macro
// live in different namespaces (type vs macro), so `use
// munibot_config::Configurable` resolves to whichever one the use site
// needs -- `#[derive(Configurable)]` finds the macro, `impl Configurable
// for X` finds the trait.
#[cfg(feature = "derive")]
pub use munibot_config_derive::{ConfigChoice, Configurable};
pub use node::{ConfigNode, ConfigSchema};
pub use patch::{ConfigChange, ConfigPatch, FieldError};
pub use path::{ConfigPath, ConfigPathError, PathSegment};
pub use refs::{ChannelRef, RoleRef, UserRef};
pub use scope::{ConfigScope, ScopeKind};
pub use toggled::Toggled;
pub use value::{ConfigValue, ConfigValues};
