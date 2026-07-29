//! Persona configuration: what makes a companion different from a researcher.
//!
//! A persona is a model, a system prompt, a tool allowlist, a budget, and an
//! optional handoff schema - the same type serves a casual chat persona and a
//! pipeline agent role alike. This is the first module in this crate that
//! depends on `munibot_core`, since its configuration section
//! hangs off `munibot_core::Config`.

pub mod template;

pub use template::PromptTemplate;
