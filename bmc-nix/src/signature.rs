// Copyright (C) 2026  Braiins Systems s.r.o.

//! Nix-style Ed25519 signatures for init tarballs.
//!
//! Keys and signatures use the nix `name:base64` line format: the
//! factory entry's `known_public_key` is the trust anchor, the feed
//! entry carries the signature, and the publisher signs with a secret
//! key in the `nix key generate-secret` layout (base64 of seed ‖
//! public key). Ed25519 signs a short domain-separated fingerprint of
//! the tarball's SHA-256 digest, never the tarball bytes, so the
//! digest can be computed incrementally while the file streams to
//! disk.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

const ED25519_PUBLIC_KEY_LEN: usize = 32;
const ED25519_SIGNATURE_LEN: usize = 64;
/// `nix key generate-secret` layout: 32-byte seed ‖ 32-byte public key.
const ED25519_SECRET_KEY_LEN: usize = 64;

/// Failure to parse a key or signature line, or a failed verification.
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("malformed public key: {0}")]
    MalformedPublicKey(String),
    // Never echo secret key material: only the failure reason.
    #[error("malformed secret key: {0}")]
    MalformedSecretKey(String),
    #[error("malformed signature: {0}")]
    MalformedSignature(String),
    #[error("signature key name '{signature}' does not match trusted key name '{trusted}'")]
    KeyNameMismatch { signature: String, trusted: String },
    #[error("Ed25519 signature verification failed for key '{key_name}'")]
    VerificationFailed { key_name: String },
}

/// Sign the fingerprint of `digest` (the tarball's SHA-256) with a
/// `name:base64(seed ‖ public key)` secret key. Returns the
/// `name:base64(signature)` line for the feed entry. This is the
/// publisher-facing entry point (bmc-packages calls it at publish
/// time).
///
/// # Errors
///
/// Returns [`SignatureError`] when the secret key line is malformed.
pub fn sign(secret_key: &str, digest: &[u8; 32]) -> Result<String, SignatureError> {
    let (name, bytes) = decode_line(secret_key, ED25519_SECRET_KEY_LEN)
        .map_err(SignatureError::MalformedSecretKey)?;
    let (seed, public_key) = bytes.split_at(ED25519_PUBLIC_KEY_LEN);
    let key_pair = ring::signature::Ed25519KeyPair::from_seed_and_public_key(seed, public_key)
        .map_err(|_| {
            SignatureError::MalformedSecretKey(
                "seed and public key halves do not form a consistent Ed25519 key pair".to_owned(),
            )
        })?;
    let signature = key_pair.sign(fingerprint(digest).as_bytes());
    Ok(format!("{name}:{}", BASE64.encode(signature.as_ref())))
}

/// Verify a `name:base64(signature)` line against the trusted
/// `name:base64(public key)` and the tarball's SHA-256 `digest`. The
/// signature's key name must equal the trusted key's name (nix's
/// rule).
///
/// # Errors
///
/// Returns [`SignatureError`] when either line is malformed, the key
/// names differ, or the Ed25519 verification fails.
pub fn verify(
    trusted_public_key: &str,
    digest: &[u8; 32],
    signature: &str,
) -> Result<(), SignatureError> {
    let (trusted_name, key_bytes) = decode_line(trusted_public_key, ED25519_PUBLIC_KEY_LEN)
        .map_err(SignatureError::MalformedPublicKey)?;
    let (signature_name, signature_bytes) = decode_line(signature, ED25519_SIGNATURE_LEN)
        .map_err(SignatureError::MalformedSignature)?;
    if signature_name != trusted_name {
        return Err(SignatureError::KeyNameMismatch {
            signature: signature_name.to_owned(),
            trusted: trusted_name.to_owned(),
        });
    }
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key_bytes)
        .verify(fingerprint(digest).as_bytes(), &signature_bytes)
        .map_err(|_| SignatureError::VerificationFailed {
            key_name: trusted_name.to_owned(),
        })
}

/// Validate a nix-format `name:base64(public key)` line without
/// verifying anything, so a caller holding a trust anchor can fail
/// fast on a malformed one before fetching what it is meant to check.
///
/// # Errors
///
/// Returns [`SignatureError::MalformedPublicKey`] when the line is not
/// a well-formed 32-byte Ed25519 public key.
pub fn validate_public_key(line: &str) -> Result<(), SignatureError> {
    decode_line(line, ED25519_PUBLIC_KEY_LEN)
        .map(|_| ())
        .map_err(SignatureError::MalformedPublicKey)
}

/// Split a nix-format `name:base64` line and decode its payload,
/// requiring exactly `expected_len` bytes.
fn decode_line(line: &str, expected_len: usize) -> Result<(&str, Vec<u8>), String> {
    let (name, encoded) = line
        .split_once(':')
        .ok_or_else(|| "missing ':' separator".to_owned())?;
    let bytes = BASE64
        .decode(encoded)
        .map_err(|err| format!("invalid base64: {err}"))?;
    if bytes.len() != expected_len {
        return Err(format!(
            "expected {expected_len} bytes, got {}",
            bytes.len()
        ));
    }
    Ok((name, bytes))
}

/// The domain-separated ASCII string Ed25519 actually signs:
/// `bmc-init-tarball-1;sha256:<64 lowercase hex chars>`. The prefix
/// separates these signatures from anything else the key might sign;
/// its `-1` suffix versions the format.
fn fingerprint(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut line = String::from("bmc-init-tarball-1;sha256:");
    for byte in digest {
        write!(line, "{byte:02x}").expect("BUG: writing to a String cannot fail");
    }
    line
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use ring::signature::KeyPair as _;

    use super::*;

    fn keypair_from_seed(name: &str, seed: &[u8; 32]) -> (String, String) {
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed)
            .expect("BUG: any 32-byte seed is a valid Ed25519 seed");
        let public = pair.public_key().as_ref().to_vec();
        let mut secret_bytes = seed.to_vec();
        secret_bytes.extend_from_slice(&public);
        (
            format!("{name}:{}", BASE64.encode(&secret_bytes)),
            format!("{name}:{}", BASE64.encode(&public)),
        )
    }

    fn test_keypair(name: &str) -> (String, String) {
        keypair_from_seed(name, &[7; 32])
    }

    #[test]
    fn sign_verify_round_trip() {
        let (secret, public) = test_keypair("braiins-init-1");
        let digest = [1_u8; 32];
        let signature = sign(&secret, &digest).expect("BUG: valid secret key");
        assert!(
            signature.starts_with("braiins-init-1:"),
            "signature line must carry the key name, got: {signature}"
        );
        verify(&public, &digest, &signature).expect("BUG: round trip must verify");
    }

    #[test]
    fn verify_rejects_tampered_digest() {
        let (secret, public) = test_keypair("k");
        let signature = sign(&secret, &[1; 32]).expect("BUG: valid secret key");
        let err = verify(&public, &[2; 32], &signature)
            .expect_err("BUG: a different digest must not verify");
        assert!(matches!(err, SignatureError::VerificationFailed { .. }));
    }

    #[test]
    fn verify_rejects_signature_by_different_key() {
        let (other_secret, _) = keypair_from_seed("k", &[8; 32]);
        let (_, public) = test_keypair("k");
        let signature = sign(&other_secret, &[1; 32]).expect("BUG: valid secret key");
        let err = verify(&public, &[1; 32], &signature)
            .expect_err("BUG: a signature by another key must not verify");
        assert!(matches!(err, SignatureError::VerificationFailed { .. }));
    }

    #[test]
    fn verify_rejects_key_name_mismatch() {
        let (secret, _) = test_keypair("one");
        let (_, public) = test_keypair("two");
        let signature = sign(&secret, &[1; 32]).expect("BUG: valid secret key");
        let err = verify(&public, &[1; 32], &signature)
            .expect_err("BUG: differing key names must be rejected");
        assert!(
            matches!(
                &err,
                SignatureError::KeyNameMismatch { signature, trusted }
                    if signature == "one" && trusted == "two"
            ),
            "expected KeyNameMismatch, got: {err:?}"
        );
    }

    #[test]
    fn validate_public_key_accepts_valid_and_rejects_malformed() {
        let (_, public) = test_keypair("k");
        validate_public_key(&public).expect("BUG: a valid public key line must validate");
        let wrong_length = format!("k:{}", BASE64.encode([0_u8; 31]));
        for line in ["nocolon", "k:!!!not-base64!!!", wrong_length.as_str()] {
            let err = validate_public_key(line)
                .expect_err("BUG: a malformed public key line must be rejected");
            assert!(
                matches!(err, SignatureError::MalformedPublicKey(_)),
                "expected MalformedPublicKey for {line:?}, got: {err:?}"
            );
        }
    }

    #[test]
    fn verify_rejects_malformed_public_keys() {
        let (secret, _) = test_keypair("k");
        let signature = sign(&secret, &[1; 32]).expect("BUG: valid secret key");
        let no_colon = "nocolon";
        let bad_base64 = "k:!!!not-base64!!!";
        let wrong_length = format!("k:{}", BASE64.encode([0_u8; 31]));
        for public in [no_colon, bad_base64, wrong_length.as_str()] {
            let err = verify(public, &[1; 32], &signature)
                .expect_err("BUG: malformed public key must be rejected");
            assert!(
                matches!(err, SignatureError::MalformedPublicKey(_)),
                "expected MalformedPublicKey for {public:?}, got: {err:?}"
            );
        }
    }

    #[test]
    fn sign_rejects_malformed_secret_keys() {
        let no_colon = "nocolon";
        let bad_base64 = "k:!!!not-base64!!!";
        let wrong_length = format!("k:{}", BASE64.encode([0_u8; 63]));
        for secret in [no_colon, bad_base64, wrong_length.as_str()] {
            let err =
                sign(secret, &[1; 32]).expect_err("BUG: malformed secret key must be rejected");
            assert!(
                matches!(err, SignatureError::MalformedSecretKey(_)),
                "expected MalformedSecretKey for a malformed input, got: {err:?}"
            );
        }
    }

    #[test]
    fn sign_rejects_inconsistent_seed_and_public_key() {
        // seed ‖ public key where the public half belongs to another
        // seed: ring must reject the pair instead of signing with it.
        let (_, other_public) = keypair_from_seed("k", &[8; 32]);
        let other_public_bytes = BASE64
            .decode(other_public.split_once(':').expect("BUG: has colon").1)
            .expect("BUG: valid base64");
        let mut secret_bytes = vec![7_u8; 32];
        secret_bytes.extend_from_slice(&other_public_bytes);
        let secret = format!("k:{}", BASE64.encode(&secret_bytes));
        let err = sign(&secret, &[1; 32])
            .expect_err("BUG: mismatched seed/public halves must be rejected");
        assert!(matches!(err, SignatureError::MalformedSecretKey(_)));
    }

    #[test]
    fn verify_rejects_malformed_signatures() {
        let (_, public) = test_keypair("k");
        let no_colon = "nocolon";
        let bad_base64 = "k:!!!not-base64!!!";
        let wrong_length = format!("k:{}", BASE64.encode([0_u8; 63]));
        for signature in [no_colon, bad_base64, wrong_length.as_str()] {
            let err = verify(&public, &[1; 32], signature)
                .expect_err("BUG: malformed signature must be rejected");
            assert!(
                matches!(err, SignatureError::MalformedSignature(_)),
                "expected MalformedSignature for {signature:?}, got: {err:?}"
            );
        }
    }

    #[test]
    fn fingerprint_is_domain_separated_and_versioned() {
        let mut digest = [0_u8; 32];
        digest[0] = 0xab;
        digest[31] = 0x01;
        assert_eq!(
            fingerprint(&digest),
            "bmc-init-tarball-1;sha256:\
             ab00000000000000000000000000000000000000000000000000000000000001"
        );
    }
}
