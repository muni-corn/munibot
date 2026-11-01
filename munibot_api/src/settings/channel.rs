use serde::{Deserialize, Serialize};

/// A discord channel, as shown in a channel picker. Deliberately slim -- just
/// enough to render a select list, grouped by category.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    /// The name of the category this channel is grouped under, if any.
    pub category: Option<String>,
}
