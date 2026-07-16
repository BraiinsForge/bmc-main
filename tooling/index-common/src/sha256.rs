// Copyright (C) 2023  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use hex::FromHex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

pub const DIGEST_LEN: usize = 32;

pub fn hash(data: impl AsRef<[u8]>) -> Sha256Digest {
    Sha256Digest::hash(data)
}

#[derive(Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Sha256Digest([u8; DIGEST_LEN]);

impl Sha256Digest {
    fn hash(data: impl AsRef<[u8]>) -> Self {
        use sha2::{Digest, Sha256};
        let d = Sha256::new().chain_update(data).finalize();
        Self(d.into())
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
}
