//! Sends transactional email over SMTP.
//!
//! The only mail munibot ever sends today is an email sign-in magic link
//! (see `oauth::email`) - this stays deliberately narrow (one method, one
//! kind of message) rather than a general-purpose templating layer, since
//! nothing else needs one yet.

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::header::ContentType,
    transport::smtp::authentication::Credentials,
};
use thiserror::Error;

/// Errors sending mail.
#[derive(Debug, Error)]
pub enum MailerError {
    #[error("couldn't build the email :< {0}")]
    Build(#[from] lettre::error::Error),
    #[error("that isn't a valid email address :< {0}")]
    InvalidAddress(#[from] lettre::address::AddressError),
    #[error("couldn't send the email :< {0}")]
    Send(#[from] lettre::transport::smtp::Error),
    #[error("couldn't set up the smtp transport :< {0}")]
    Transport(String),
}

/// An SMTP relay munibot sends mail through.
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl Mailer {
    /// Builds a mailer from `SMTP_HOST`, `SMTP_USERNAME`, `SMTP_PASSWORD`,
    /// and `SMTP_FROM_ADDRESS` - `SMTP_PORT` optionally, defaulting to
    /// `587` (STARTTLS submission, the common default for a relay).
    ///
    /// Returns `None` (never an error) when `SMTP_HOST` isn't set - email
    /// sign-in is opt-in infrastructure, the same reasoning
    /// `crate::oauth::github`'s own client id/secret being optional
    /// documents, not something that should fail startup over.
    pub fn from_env() -> Option<Result<Self, MailerError>> {
        let host = std::env::var("SMTP_HOST").ok()?;
        Some(Self::build(&host))
    }

    fn build(host: &str) -> Result<Self, MailerError> {
        let username = std::env::var("SMTP_USERNAME").unwrap_or_default();
        let password = std::env::var("SMTP_PASSWORD").unwrap_or_default();
        let from = std::env::var("SMTP_FROM_ADDRESS")
            .unwrap_or_else(|_| format!("munibot <no-reply@{host}>"));
        let port: u16 = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(587);

        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|error| MailerError::Transport(error.to_string()))?
            .port(port);

        if !username.is_empty() {
            builder = builder.credentials(Credentials::new(username, password));
        }

        Ok(Self {
            transport: builder.build(),
            from,
        })
    }

    /// Sends a sign-in magic link to `to`. `link` is the fully-formed
    /// callback URL (already carrying its own token), rendered verbatim -
    /// see `oauth::email::request_signin`, the only caller.
    pub async fn send_signin_link(&self, to: &str, link: &str) -> Result<(), MailerError> {
        let body = format!(
            "hi! click this link to sign in to munibot:\n\n{link}\n\nthis link works once, and \
             stops working in 15 minutes. if you didn't ask for this, you can just ignore it."
        );

        let email = Message::builder()
            .from(self.from.parse()?)
            .to(to.parse()?)
            .subject("sign in to munibot")
            .header(ContentType::TEXT_PLAIN)
            .body(body)?;

        self.transport.send(email).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_is_none_without_smtp_host() {
        // SAFETY: test-only, and no other test in this process reads or
        // writes SMTP_HOST concurrently with this one - the same pattern
        // OpenAiModerator::from_env's own test uses for OPENAI_API_KEY
        unsafe {
            std::env::remove_var("SMTP_HOST");
        }
        assert!(Mailer::from_env().is_none());
    }
}
