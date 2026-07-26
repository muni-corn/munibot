//! Derive macros for `munibot_config`: `Configurable` and `ConfigChoice`.
//!
//! See docs/plans/gui-configuration.md section 6 and
//! docs/notes/config-schema-implementation-notes.md for the design this
//! implements, including several v1 scope decisions (e.g. only three field
//! shapes are recognized: `Toggled<T>`, `BTreeMap<K, V>`, and everything
//! else) made while writing this crate.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod attrs;
mod choice;
mod configurable;
mod shape;

/// Derives `munibot_config::Configurable` for a struct.
///
/// See the crate-level docs for the attributes this supports.
#[proc_macro_derive(Configurable, attributes(config))]
pub fn derive_configurable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    configurable::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derives `munibot_config::ConfigChoice` (plus `ConfigField`,
/// `Serialize`, and `Deserialize`) for a fieldless enum.
///
/// See the crate-level docs for the attributes this supports.
#[proc_macro_derive(ConfigChoice, attributes(config))]
pub fn derive_config_choice(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    choice::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
