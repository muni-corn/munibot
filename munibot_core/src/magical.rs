use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use chrono::Local;

/// Computes a deterministic magicalness percentage (0–100) for a user on the
/// current calendar day. The result is stable for a given user ID and date.
pub fn get_magic_amount(user_id: &str) -> u8 {
    // we determine a user's magicalness based on the current date and their user
    // id.
    let today = Local::now().date_naive();

    // hash the value
    let mut hash_state = DefaultHasher::new();
    (today, user_id).hash(&mut hash_state);
    let hashed = hash_state.finish();

    // a number between 0 and 100
    let x = hashed % 101;

    // give a cubic-interpolated value between 1 and 100, favoring higher numbers,
    // without floating point arithmetic :>
    ((100u64.pow(3) - x * x * x) / (100 * 100)) as u8
}

/// Returns a human-readable magicalness message for the given user.
pub fn get_magic_message(user_id: &str, user_display_name: &str) -> String {
    let magic_amount = get_magic_amount(user_id);
    let suffix = match magic_amount {
        x if x <= 1 => ". you can have some of my magic~ :3 <3",
        x if x < 25 => ". sounds like a good day for some self care <3",
        69 => ". nice ;3",
        x if x < 75 => ".",
        100 => "!! wow :3",
        _ => "!",
    };
    format!("{user_display_name} is {magic_amount}% magical today{suffix}")
}
