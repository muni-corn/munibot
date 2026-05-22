use once_cell::sync::Lazy;
use rand::seq::IndexedRandom;
use regex::Regex;

static HI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:hi+|hey+|hello+|howdy+|sup+|heww?o+|henlo+)\b.*\bmuni.?bot\b").unwrap()
});

const HELLO_TEMPLATES: [&str; 11] = [
    "hi, {name}!<3",
    "hello, {name}! happy to see you!",
    "hey {name}:)",
    "hi {name}!! how are ya?",
    "{name}!! how are you doing?",
    "heyyy {name} uwu",
    "hi {name}! it's good to see you! :3",
    "{name} helloooooo:)",
    "hiiiii {name}",
    "hi {name}<3",
    "hi {name}! you look wonderful today ;3",
];

/// Returns `true` if the given message text matches the greeting pattern.
pub fn matches_greeting(message_text: &str) -> bool {
    HI_REGEX.is_match(message_text)
}

/// Returns a greeting response for the given user if the message matches the
/// greeting pattern, or `None` if the message should be ignored.
pub fn get_greeting_response(user_name: &str, message_text: &str) -> Option<String> {
    if matches_greeting(message_text) {
        let mut rng = rand::rng();
        let greeting = HELLO_TEMPLATES
            .choose(&mut rng)
            .unwrap()
            .replace("{name}", user_name);
        Some(greeting)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hi_munibot_matches() {
        let result = get_greeting_response("alice", "hi munibot");
        assert!(result.is_some(), "expected greeting for 'hi munibot'");
    }

    #[test]
    fn test_hello_munibot_matches() {
        let result = get_greeting_response("alice", "hello munibot");
        assert!(result.is_some(), "expected greeting for 'hello munibot'");
    }

    #[test]
    fn test_hey_munibot_matches() {
        let result = get_greeting_response("alice", "hey munibot");
        assert!(result.is_some(), "expected greeting for 'hey munibot'");
    }

    #[test]
    fn test_hewwo_munibot_matches() {
        let result = get_greeting_response("alice", "hewwo munibot");
        assert!(result.is_some(), "expected greeting for 'hewwo munibot'");
    }

    #[test]
    fn test_henlo_munibot_matches() {
        let result = get_greeting_response("alice", "henlo munibot");
        assert!(result.is_some(), "expected greeting for 'henlo munibot'");
    }

    #[test]
    fn test_case_insensitive_match() {
        let result = get_greeting_response("alice", "HI MUNIBOT");
        assert!(result.is_some(), "expected greeting to be case-insensitive");
    }

    #[test]
    fn test_words_between_hi_and_munibot() {
        // regex allows words between the greeting and "munibot"
        let result = get_greeting_response("alice", "hi there munibot!");
        assert!(result.is_some(), "expected greeting with words in between");
    }

    #[test]
    fn test_no_match_without_munibot() {
        let result = get_greeting_response("alice", "hi everyone");
        assert!(result.is_none(), "expected no greeting without 'munibot'");
    }

    #[test]
    fn test_no_match_unrelated_message() {
        let result = get_greeting_response("alice", "how are you doing?");
        assert!(
            result.is_none(),
            "expected no greeting for unrelated message"
        );
    }

    #[test]
    fn test_no_match_empty_string() {
        let result = get_greeting_response("alice", "");
        assert!(result.is_none(), "expected no greeting for empty message");
    }

    #[test]
    fn test_response_contains_username() {
        let result = get_greeting_response("alice", "hi munibot");
        let msg = result.expect("expected a greeting message");
        assert!(
            msg.contains("alice"),
            "expected response to contain username, got '{msg}'"
        );
    }

    #[test]
    fn test_different_users_get_their_name() {
        let result = get_greeting_response("bobcat", "hello munibot");
        let msg = result.expect("expected a greeting message");
        assert!(
            msg.contains("bobcat"),
            "expected response to contain 'bobcat', got '{msg}'"
        );
    }
}
