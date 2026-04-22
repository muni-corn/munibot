use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
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
        let mut rng = rand::thread_rng();
        let greeting = HELLO_TEMPLATES
            .choose(&mut rng)
            .unwrap()
            .replace("{name}", user_name);
        Some(greeting)
    } else {
        None
    }
}
