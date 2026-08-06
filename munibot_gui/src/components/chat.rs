//! Chat-specific rendering primitives, new ground unlike the settings design
//! system in `crate::components::settings`: markdown rendering, the message
//! list, and (in later commits) the composer and tool activity display.

pub mod composer;
pub mod markdown;
pub mod message_list;
pub mod persona_picker;
pub mod tool_activity;
pub mod turn_failure;
