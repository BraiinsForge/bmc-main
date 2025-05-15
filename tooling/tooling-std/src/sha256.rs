// Copyright (C) 2023  Braiins Systems s.r.o.

use anyhow::Result;
use bincode::{Decode, Encode};
use hex::FromHex;
use rand::RngCore;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

pub const DIGEST_LEN: usize = 32;

pub fn hash(data: impl AsRef<[u8]>) -> Sha256Digest {
    Sha256Digest::hash(data)
}

#[derive(Encode, Decode, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Sha256Digest([u8; DIGEST_LEN]);

impl Sha256Digest {
    fn hash(data: impl AsRef<[u8]>) -> Self {
        use sha2::{Digest, Sha256};
        let d = Sha256::new().chain_update(data).finalize();
        Self(d.into())
    }

    pub fn new_random(rng: &mut impl RngCore) -> Self {
        let mut nonce = [0_u8; DIGEST_LEN];
        rng.fill_bytes(&mut nonce);
        Self(nonce)
    }

    pub fn from_sha256sum_output(data: impl AsRef<[u8]>) -> Result<Self> {
        const HEX_LEN: usize = 64;
        let hex = data.as_ref().get(0..HEX_LEN).unwrap_or_default();
        let hex = <[u8; HEX_LEN]>::try_from(hex)?;
        let digest = <[u8; DIGEST_LEN]>::from_hex(hex)?;
        Ok(Self(digest))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }
}

impl FromStr for Sha256Digest {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digest = <[u8; DIGEST_LEN]>::from_hex(s)?;
        Ok(Self(digest))
    }
}

impl Display for Sha256Digest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl Debug for Sha256Digest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn serialize() {
        let digest = hash("");
        let json = serde_json::to_string(&digest).unwrap();
        assert_eq!(
            r#""e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855""#,
            json
        );
    }

    #[test]
    fn deserialize() {
        let digest = hash("");
        let json = serde_json::from_str::<Sha256Digest>(
            r#""e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855""#,
        )
        .unwrap();
        assert_eq!(json, digest);
    }

    #[test]
    fn from_sha256sum_file_success() {
        let sha256sum_output = r"dd0ba065b7d1609b08b8a6f7149e88bb9131a372b976b8a3a0bf955a6d393355  braiins-os_am1-s9_sd-install_2023-01-26-0-77b82ea2-23.02-plus-nightly.img";

        let result = Sha256Digest::from_sha256sum_output(sha256sum_output);

        let expected =
            hex::decode("dd0ba065b7d1609b08b8a6f7149e88bb9131a372b976b8a3a0bf955a6d393355")
                .expect("BUG: cannot decode hex");

        let digest = assert_matches!(result, Ok(digest) => digest);
        assert_eq!(digest.as_slice(), expected.as_slice());
    }

    #[test]
    fn from_sha256sum_file_failure() {
        let checksum_response = r"invalid-response";

        let result = Sha256Digest::from_sha256sum_output(checksum_response);
        assert_matches!(result, Err(_));
    }
}
