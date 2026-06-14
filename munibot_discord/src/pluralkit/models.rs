use serde::Deserialize;

/// The system that sent a proxied message.
#[derive(Clone, Debug, Deserialize)]
pub struct PkSystem {
    /// The system's short (5-6 character) ID.
    pub id: String,

    /// The system's display name, if set.
    pub name: Option<String>,
}

/// The member that sent a proxied message.
#[derive(Clone, Debug, Deserialize)]
pub struct PkMember {
    /// The member's short (5-6 character) ID.
    pub id: String,

    /// The member's name.
    pub name: String,

    /// The member's display name, if set. Prefer this over `name` for display.
    pub display_name: Option<String>,
}

impl PkMember {
    /// Returns the display name if set, otherwise the member name.
    pub fn display(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

/// A proxied message as returned by the PluralKit API.
///
/// Corresponds to the message object at `GET /messages/{id}`.
#[derive(Clone, Debug, Deserialize)]
pub struct PkMessage {
    /// The ID of the webhook message PluralKit sent (the proxy).
    pub id: String,

    /// The ID of the original message that triggered the proxy (now deleted).
    pub original: String,

    /// The Discord user ID of the account that triggered the proxy.
    pub sender: String,

    /// The system that proxied the message. Null if the member was deleted.
    pub system: Option<PkSystem>,

    /// The member that proxied the message. Null if the member was deleted.
    pub member: Option<PkMember>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample response from `GET /messages/{id}` with all fields present.
    const FULL_RESPONSE: &str = r#"{
        "timestamp": "2024-01-15T12:34:56.789Z",
        "id": "1196543210987654321",
        "original": "1196543210987654000",
        "sender": "123456789012345678",
        "channel": "987654321098765432",
        "guild": "111222333444555666",
        "system": {
            "id": "abcde",
            "uuid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "name": "The Starlight Collective",
            "description": null,
            "tag": null,
            "pronouns": null,
            "avatar_url": null,
            "banner": null,
            "color": null,
            "created": "2021-03-01T00:00:00.000Z"
        },
        "member": {
            "id": "fghij",
            "uuid": "ffffffff-gggg-hhhh-iiii-jjjjjjjjjjjj",
            "name": "Lyra",
            "display_name": "Lyra Starweaver",
            "color": null,
            "birthday": null,
            "pronouns": "she/her",
            "avatar_url": null,
            "webhook_avatar_url": null,
            "banner": null,
            "description": null,
            "created": "2021-03-01T00:00:00.000Z",
            "keep_proxy": false,
            "tts": false,
            "autoproxy_enabled": null,
            "message_count": 42,
            "last_message_timestamp": "2024-01-15T12:34:56.789Z",
            "proxy_tags": [{"prefix": "[", "suffix": "]"}],
            "privacy": null
        }
    }"#;

    /// Sample response where the member and system have been deleted.
    const DELETED_MEMBER_RESPONSE: &str = r#"{
        "timestamp": "2024-01-15T12:34:56.789Z",
        "id": "1196543210987654321",
        "original": "1196543210987654000",
        "sender": "123456789012345678",
        "channel": "987654321098765432",
        "guild": "111222333444555666",
        "system": null,
        "member": null
    }"#;

    #[test]
    fn test_deserialize_full_response() {
        let msg: PkMessage =
            serde_json::from_str(FULL_RESPONSE).expect("should deserialize full response");

        assert_eq!(msg.id, "1196543210987654321");
        assert_eq!(msg.original, "1196543210987654000");
        assert_eq!(msg.sender, "123456789012345678");

        let system = msg.system.expect("system should be present");
        assert_eq!(system.id, "abcde");
        assert_eq!(system.name.as_deref(), Some("The Starlight Collective"));

        let member = msg.member.expect("member should be present");
        assert_eq!(member.id, "fghij");
        assert_eq!(member.name, "Lyra");
        assert_eq!(member.display_name.as_deref(), Some("Lyra Starweaver"));
        assert_eq!(member.display(), "Lyra Starweaver");
    }

    #[test]
    fn test_deserialize_deleted_member_response() {
        let msg: PkMessage =
            serde_json::from_str(DELETED_MEMBER_RESPONSE).expect("should deserialize");

        assert_eq!(msg.id, "1196543210987654321");
        assert!(msg.system.is_none(), "system should be null");
        assert!(msg.member.is_none(), "member should be null");
    }

    #[test]
    fn test_member_display_falls_back_to_name() {
        let member = PkMember {
            id: "abcde".into(),
            name: "Lyra".into(),
            display_name: None,
        };
        assert_eq!(member.display(), "Lyra");
    }

    #[test]
    fn test_member_display_prefers_display_name() {
        let member = PkMember {
            id: "abcde".into(),
            name: "Lyra".into(),
            display_name: Some("Lyra Starweaver".into()),
        };
        assert_eq!(member.display(), "Lyra Starweaver");
    }
}
