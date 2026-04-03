use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("missing X-Hub-Signature-256 header")]
    Missing,
    #[error("invalid signature format (expected 'sha256=<hex>')")]
    InvalidFormat,
    #[error("signature mismatch")]
    Mismatch,
}

/// Verify a GitHub webhook HMAC-SHA256 signature.
///
/// `signature_header` is the raw value of `X-Hub-Signature-256`
/// (e.g. `sha256=abc123...`).
///
/// # Errors
/// Returns [`SignatureError`] if the signature is missing, malformed, or does not match.
pub fn verify(secret: &str, body: &[u8], signature_header: &str) -> Result<(), SignatureError> {
    let hex_sig = signature_header
        .strip_prefix("sha256=")
        .ok_or(SignatureError::InvalidFormat)?;

    let expected = hex::decode(hex_sig).map_err(|_| SignatureError::InvalidFormat)?;

    // HMAC<Sha256> accepts any key length; the only error case is zero-length key
    // which cannot happen here because &str references are non-null (even if empty,
    // HMAC still accepts it). We use `unwrap_or` with a compile-time known-safe path.
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| SignatureError::InvalidFormat)?;
    mac.update(body);

    mac.verify_slice(&expected)
        .map_err(|_| SignatureError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature_passes() {
        let secret = "mysecret";
        let body = b"hello world";

        #[allow(clippy::unwrap_used)]
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let sig = format!("sha256={}", hex::encode(result));

        assert!(verify(secret, body, &sig).is_ok());
    }

    #[test]
    fn wrong_signature_fails() {
        let result = verify("secret", b"body", "sha256=deadbeef");
        assert!(matches!(
            result,
            Err(SignatureError::InvalidFormat) | Err(SignatureError::Mismatch)
        ));
    }

    #[test]
    fn missing_prefix_fails() {
        let result = verify("secret", b"body", "abc123");
        assert!(matches!(result, Err(SignatureError::InvalidFormat)));
    }
}
