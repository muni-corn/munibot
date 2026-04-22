// Discord-only handlers (in munibot_discord)
// Collection type aliases
pub use munibot_discord::{
    commands::DiscordCommandProviderCollection,
    handlers::{bot_affection, dice, economy, eight_ball, temperature, ventriloquize},
    state::DiscordMessageHandlerCollection,
};
pub use munibot_twitch::handler::TwitchHandlerCollection;
// Twitch-only handlers (in munibot_twitch)
pub use munibot_twitch::handlers::{
    affection, autoban, bonk, content_warning, greeting, lift, lurk, magical, quotes, shoutout,
    socials,
};
