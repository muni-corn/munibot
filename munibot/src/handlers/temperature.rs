use crate::{
    MuniBotError,
    discord::{DiscordCommand, DiscordContext, commands::DiscordCommandProvider},
};

pub struct TemperatureConversionProvider;

/// Convert temperatures between Fahrenheit and Celsius.
#[poise::command(prefix_command, track_edits, slash_command)]
async fn convert_temperature(
    ctx: DiscordContext<'_>,
    #[description = "temperature to convert, ending in 'F' or 'C'"] temperature: String,
) -> Result<(), MuniBotError> {
    let temperature = temperature.to_string().trim().to_lowercase();

    let quantity = temperature
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect::<String>()
        .parse::<f32>()
        .map_err(|e| MuniBotError::Other(format!("couldn't parse temperature: {e}")))?;

    let unit = temperature.chars().find(|c| *c == 'f' || *c == 'c');

    let response = match unit {
        Some('f') => get_fahrenheit_to_celsius_message(quantity),
        Some('c') => get_celsius_to_fahrenheit_message(quantity),
        None => {
            let c_to_f = get_celsius_to_fahrenheit_message(quantity);
            let f_to_c = get_fahrenheit_to_celsius_message(quantity);
            format!("{c_to_f} or {f_to_c}")
        }
        _ => unreachable!(),
    };

    ctx.say(response).await.map_err(|e| {
        MuniBotError::Other(format!(
            "couldn't send temperature conversion response: {e}"
        ))
    })?;

    Ok(())
}

fn get_fahrenheit_to_celsius_message(fahrenheit: f32) -> String {
    format!(
        "{fahrenheit}°F is {:.1}°C :3",
        (fahrenheit - 32.0) * 5.0 / 9.0
    )
}

fn get_celsius_to_fahrenheit_message(celsius: f32) -> String {
    format!("{celsius}°C is {:.0}°F :3", (celsius * 9.0 / 5.0) + 32.0)
}

impl DiscordCommandProvider for TemperatureConversionProvider {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![convert_temperature()]
    }
}
