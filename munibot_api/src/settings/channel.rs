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

#[cfg(feature = "server")]
mod sort {
    use std::collections::HashMap;

    use crate::{oauth::discord::bot, settings::ChannelSummary};

    /// Picks out `guild`'s text-postable channels and orders them the way
    /// discord's own client does: uncategorized channels first, then each
    /// channel by its position within its category, with categories
    /// ordered by their own position among other categories.
    ///
    /// A channel's raw `position` alone isn't enough to sort by -- it only
    /// orders channels within the same parent, so sorting by it directly
    /// would interleave categories.
    pub fn sort_text_channels(channels: &[bot::DiscordChannel]) -> Vec<ChannelSummary> {
        let category_positions: HashMap<&str, i32> = channels
            .iter()
            .filter(|channel| channel.kind == bot::CHANNEL_TYPE_CATEGORY)
            .map(|channel| (channel.id.as_str(), channel.position))
            .collect();
        let category_names: HashMap<&str, &str> = channels
            .iter()
            .filter(|channel| channel.kind == bot::CHANNEL_TYPE_CATEGORY)
            .map(|channel| (channel.id.as_str(), channel.name.as_str()))
            .collect();

        let mut text_channels: Vec<_> = channels
            .iter()
            .filter(|channel| {
                matches!(
                    channel.kind,
                    bot::CHANNEL_TYPE_TEXT | bot::CHANNEL_TYPE_ANNOUNCEMENT
                )
            })
            .collect();
        text_channels.sort_by_key(|channel| {
            let category_position = channel
                .parent_id
                .as_deref()
                .and_then(|id| category_positions.get(id))
                .copied()
                .unwrap_or(i32::MIN);
            (category_position, channel.position)
        });

        text_channels
            .into_iter()
            .map(|channel| ChannelSummary {
                id: channel.id.clone(),
                name: channel.name.clone(),
                category: channel
                    .parent_id
                    .as_deref()
                    .and_then(|id| category_names.get(id))
                    .map(|name| name.to_string()),
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn channel(
            id: &str,
            kind: u8,
            name: &str,
            parent_id: Option<&str>,
            position: i32,
        ) -> bot::DiscordChannel {
            bot::DiscordChannel {
                id: id.to_string(),
                kind,
                name: name.to_string(),
                parent_id: parent_id.map(str::to_string),
                position,
            }
        }

        #[test]
        fn orders_uncategorized_channels_first() {
            let channels = vec![
                channel("cat", bot::CHANNEL_TYPE_CATEGORY, "category", None, 0),
                channel(
                    "in-cat",
                    bot::CHANNEL_TYPE_TEXT,
                    "in category",
                    Some("cat"),
                    0,
                ),
                channel(
                    "uncategorized",
                    bot::CHANNEL_TYPE_TEXT,
                    "uncategorized",
                    None,
                    5,
                ),
            ];

            let sorted = sort_text_channels(&channels);

            assert_eq!(
                sorted.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                vec!["uncategorized", "in-cat"]
            );
        }

        #[test]
        fn orders_categories_by_their_own_position() {
            let channels = vec![
                channel("cat-b", bot::CHANNEL_TYPE_CATEGORY, "b", None, 1),
                channel("cat-a", bot::CHANNEL_TYPE_CATEGORY, "a", None, 0),
                channel("in-b", bot::CHANNEL_TYPE_TEXT, "in b", Some("cat-b"), 0),
                channel("in-a", bot::CHANNEL_TYPE_TEXT, "in a", Some("cat-a"), 0),
            ];

            let sorted = sort_text_channels(&channels);

            assert_eq!(
                sorted.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                vec!["in-a", "in-b"]
            );
        }

        #[test]
        fn orders_channels_within_a_category_by_position() {
            let channels = vec![
                channel("cat", bot::CHANNEL_TYPE_CATEGORY, "category", None, 0),
                channel("second", bot::CHANNEL_TYPE_TEXT, "second", Some("cat"), 1),
                channel("first", bot::CHANNEL_TYPE_TEXT, "first", Some("cat"), 0),
            ];

            let sorted = sort_text_channels(&channels);

            assert_eq!(
                sorted.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                vec!["first", "second"]
            );
        }

        #[test]
        fn excludes_categories_and_non_text_channels() {
            let channels = vec![
                channel("cat", bot::CHANNEL_TYPE_CATEGORY, "category", None, 0),
                channel("voice", 2, "voice", None, 1),
                channel("text", bot::CHANNEL_TYPE_TEXT, "text", None, 2),
            ];

            let sorted = sort_text_channels(&channels);

            assert_eq!(
                sorted.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                vec!["text"]
            );
        }

        #[test]
        fn includes_announcement_channels() {
            let channels = vec![channel(
                "announcement",
                bot::CHANNEL_TYPE_ANNOUNCEMENT,
                "announcements",
                None,
                0,
            )];

            let sorted = sort_text_channels(&channels);

            assert_eq!(sorted.len(), 1);
        }

        #[test]
        fn attaches_the_containing_category_name() {
            let channels = vec![
                channel("cat", bot::CHANNEL_TYPE_CATEGORY, "general chat", None, 0),
                channel("text", bot::CHANNEL_TYPE_TEXT, "text", Some("cat"), 0),
            ];

            let sorted = sort_text_channels(&channels);

            assert_eq!(sorted[0].category.as_deref(), Some("general chat"));
        }
    }
}

#[cfg(feature = "server")]
pub use sort::sort_text_channels;
