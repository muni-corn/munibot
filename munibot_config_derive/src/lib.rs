//! Derive macros for `munibot_config`: `Configurable` and `ConfigChoice`.
//!
//! See docs/plans/gui-configuration.md section 6 and
//! docs/notes/config-schema-implementation-notes.md for the design this
//! implements, including several v1 scope decisions (e.g. only three field
//! shapes are recognized: `Toggled<T>`, `BTreeMap<K, V>`, and everything
//! else) made while writing this crate.

use proc_macro::TokenStream;

mod attrs;

/// Derives `munibot_config::Configurable` for a struct.
///
/// See the crate-level docs for the attributes this supports.
#[proc_macro_derive(Configurable, attributes(config))]
pub fn derive_configurable(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

/// Derives `munibot_config::ConfigChoice` (plus `ConfigField`,
/// `Serialize`, and `Deserialize`) for a fieldless enum.
///
/// See the crate-level docs for the attributes this supports.
#[proc_macro_derive(ConfigChoice, attributes(config))]
pub fn derive_config_choice(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
