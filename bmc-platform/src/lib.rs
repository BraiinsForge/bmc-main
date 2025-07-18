// Copyright (C) 2025  Braiins Systems s.r.o.

use index_bmc::BmcPlatform as IndexBmcPlatform;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, io};
use strum::{EnumIter, EnumMessage, EnumString};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LoadError {
    #[error("I/O error loading {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Parse(#[from] strum::ParseError),
}

#[derive(Debug, Clone, Copy, EnumString, Eq, PartialEq, EnumMessage, EnumIter, Hash)]
pub enum BmcPlatform {
    #[strum(serialize = "stm32mp157c-ii3-bmc1")]
    BraiinsBmc,
}

impl Display for BmcPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Write first serialization, if available (should be always true).
        // In an unlikely case of no serialization, report the platform as unknown.
        write!(
            f,
            "{}",
            self.get_serializations().first().unwrap_or(&"unknown")
        )
    }
}

impl From<BmcPlatform> for IndexBmcPlatform {
    fn from(value: BmcPlatform) -> Self {
        match value {
            BmcPlatform::BraiinsBmc => IndexBmcPlatform::Stm32mp157cIi3Bmc1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BosVersion {
    pub full: String,
    pub major: String,
    pub is_bos_plus: bool,
}

impl BosVersion {
    const BOS_PLUS_SUFFIX: &str = "-plus";

    pub fn new<T: ToString>(full: &T, major: &T) -> Self {
        // TODO: do better parsing of BOS version
        let full = full.to_string();
        let major = major.to_string();
        Self {
            is_bos_plus: full.contains(Self::BOS_PLUS_SUFFIX),
            full,
            major,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BmcInfo {
    pub bmc_platform: BmcPlatform,
    pub bos_version: BosVersion,
}

impl BmcInfo {
    const BOS_PLATFORM_PATH: &str = "etc/bos_platform";
    const BOS_VERSION_PATH: &str = "etc/bos_version";
    const BOS_MAJOR_VERSION_PATH: &str = "etc/bos_major";

    /// Standard BosInfo loader that ignores any path prefix
    pub fn load() -> Result<Self, LoadError> {
        Self::load_with_path_prefix::<&Path>(None)
    }

    /// Optional `path_prefix` to be appended to the configuration path of each element
    pub fn load_with_path_prefix<P: AsRef<Path>>(
        path_prefix: Option<P>,
    ) -> Result<Self, LoadError> {
        let path_prefix = path_prefix
            .map_or(PathBuf::from(std::path::MAIN_SEPARATOR.to_string()), |p| {
                p.as_ref().to_owned()
            });
        Ok(Self {
            bmc_platform: BmcPlatform::from_str(&Self::read_to_string(
                &path_prefix,
                Self::BOS_PLATFORM_PATH,
            )?)?,
            bos_version: BosVersion::new(
                &Self::read_to_string(&path_prefix, Self::BOS_VERSION_PATH)?,
                &Self::read_to_string(&path_prefix, Self::BOS_MAJOR_VERSION_PATH)
                    .unwrap_or_default(),
            ),
        })
    }

    fn read_to_string(
        path_prefix: impl AsRef<Path>,
        info_path: &'static str,
    ) -> Result<String, LoadError> {
        let final_path = path_prefix.as_ref().join(info_path);
        fs::read_to_string(&final_path)
            .map(|s| s.trim().to_owned())
            .map_err(|e| LoadError::Io {
                source: e,
                path: final_path,
            })
    }

    #[must_use]
    pub fn new(bmc_platform: BmcPlatform, bos_version: BosVersion) -> Self {
        Self {
            bmc_platform,
            bos_version,
        }
    }

    #[inline]
    #[must_use]
    pub fn is_bos_plus(&self) -> bool {
        self.bos_version.is_bos_plus
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn bos_platform_deserialization_test() {
        let test_data = [(BmcPlatform::BraiinsBmc, "stm32mp157c-ii3-bmc1")];
        #[expect(clippy::expect_fun_call)]
        for (platform, string_platform) in test_data {
            assert_eq!(
                platform,
                BmcPlatform::from_str(string_platform).expect(
                    format!("BUG: Deserialization of value {string_platform:?} failed").as_str()
                ),
                "BUG: BOS platform does not match"
            );
        }
    }
}
