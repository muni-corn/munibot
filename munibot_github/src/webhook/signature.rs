//! Verifying that a webhook delivery actually came from GitHub.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::GitHubError;

type HmacSha256 = Hmac<Sha256>;

/// Verifies the `X-Hub-Signature-256` header GitHub sends with every
/// webhook delivery against `secret` and the **raw** request body.
///
/// Must run over the raw bytes, before any JSON parsing: the signature
/// covers exactly the bytes GitHub sent over the wire, and re-serializing a
/// parsed payload would not reproduce them byte for byte. `header` is
/// `None` when the request carried no `X-Hub-Signature-256` header at
/// all, which is rejected the same as a malformed one -- there is no
/// legitimate GitHub delivery without this header once a webhook secret is
/// configured.
///
/// The comparison itself is constant-time (`subtle::ConstantTimeEq`), not
/// as a nicety: a timing side channel here would let an attacker recover a
/// valid signature one byte at a time and forge webhook deliveries that
/// reach an agent holding filesystem and shell tools.
pub fn verify_signature(
    secret: &str,
    raw_body: &[u8],
    header: Option<&str>,
) -> Result<(), GitHubError> {
    let header = header
        .ok_or_else(|| GitHubError::Auth("webhook delivery has no signature header".to_string()))?;

    let hex_digest = header.strip_prefix("sha256=").ok_or_else(|| {
        GitHubError::Auth("webhook signature header is missing its sha256= prefix".to_string())
    })?;

    let expected = decode_hex(hex_digest).ok_or_else(|| {
        GitHubError::Auth("webhook signature header is not valid hex".to_string())
    })?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|error| GitHubError::Config(format!("invalid webhook secret: {error}")))?;
    mac.update(raw_body);
    let computed = mac.finalize().into_bytes();

    if bool::from(computed.as_slice().ct_eq(&expected)) {
        Ok(())
    } else {
        Err(GitHubError::Auth(
            "webhook signature does not match the payload".to_string(),
        ))
    }
}

/// Decodes a lowercase or uppercase hex string into bytes, returning `None`
/// for an odd length or any non-hex character rather than panicking --
/// this parses attacker-controlled header text, so a malformed value must
/// be an ordinary rejected signature, never a crash.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }

    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // computed with `openssl dgst -sha256 -hmac "it's a secret to everybody"`
    // over the exact body below -- a known-good vector, not a placeholder
    const SECRET: &str = "it's a secret to everybody";
    const BODY: &[u8] = br#"{"zen":"test"}"#;
    const VALID_SIGNATURE: &str =
        "sha256=cec9672f9a62d6f0a085431adb74f116c1efbf170a80f0b4bb81ff59da31bc20";

    #[test]
    fn test_verify_signature_accepts_a_known_good_vector() {
        assert!(verify_signature(SECRET, BODY, Some(VALID_SIGNATURE)).is_ok());
    }

    #[test]
    fn test_verify_signature_rejects_a_tampered_body() {
        let result = verify_signature(SECRET, br#"{"zen":"tampered"}"#, Some(VALID_SIGNATURE));
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_signature_rejects_the_wrong_secret() {
        let result = verify_signature("wrong secret", BODY, Some(VALID_SIGNATURE));
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_signature_rejects_a_missing_header() {
        let result = verify_signature(SECRET, BODY, None);
        let error = result.expect_err("a missing signature header must be rejected");
        assert!(error.to_string().contains("no signature header"));
    }

    #[test]
    fn test_verify_signature_rejects_a_header_missing_the_sha256_prefix() {
        let bare_hex = VALID_SIGNATURE.strip_prefix("sha256=").unwrap();
        let result = verify_signature(SECRET, BODY, Some(bare_hex));
        let error = result.expect_err("a header without the sha256= prefix must be rejected");
        assert!(error.to_string().contains("sha256="));
    }

    #[test]
    fn test_verify_signature_rejects_non_hex_garbage() {
        let result = verify_signature(SECRET, BODY, Some("sha256=not-hex-at-all!!"));
        let error = result.expect_err("non-hex signature text must be rejected");
        assert!(error.to_string().contains("not valid hex"));
    }

    #[test]
    fn test_verify_signature_rejects_a_short_signature() {
        let result = verify_signature(SECRET, BODY, Some("sha256=abcd"));
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_hex_rejects_an_odd_length_string() {
        assert_eq!(decode_hex("abc"), None);
    }

    #[test]
    fn test_decode_hex_rejects_non_hex_characters() {
        assert_eq!(decode_hex("zz"), None);
    }

    #[test]
    fn test_decode_hex_decodes_a_valid_string() {
        assert_eq!(decode_hex("00ff"), Some(vec![0x00, 0xff]));
    }
}
