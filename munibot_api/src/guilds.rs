use serde::{Deserialize, Serialize};

/// A discord guild (server) the signed-in user owns or can manage, shown on
/// the dashboard. Deliberately slim -- just enough to render a list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GuildSummary {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
}
