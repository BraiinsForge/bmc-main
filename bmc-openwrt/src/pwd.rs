// Copyright (C) 2025  Braiins Systems s.r.o.

use pw_hash::{md5_crypt, sha512_crypt, unix};
use std::fs::File;
use std::io;
use std::path::Path;
use std::str::FromStr;

pub const SHADOW_PATH: &str = "/etc/shadow";

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Hash(#[from] pw_hash::error::Error),
}

#[derive(Debug, Clone)]
pub struct ShadowLine {
    username: String,
    password_hash: Option<String>,
    last_change: Option<u32>,
    min_days: Option<u32>,
    max_days: Option<u32>,
    warn_days: Option<u32>,
    inactive_days: Option<u32>,
    expire_days: Option<u32>,
}

/// Defines hashing algorithm for setting of new password
#[allow(dead_code, clippy::allow_attributes)]
#[derive(Debug, Clone, Copy)]
pub enum PasswordHashType {
    Sha512,
    Md5,
}

#[derive(Debug)]
pub struct ShadowFile {
    lines: Vec<ShadowLine>,
}

/// Implement `Read` trait for `ShadowFile` struct
impl io::Read for ShadowFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let shadow_file = self.to_string();
        shadow_file.as_bytes().read(buf)
    }
}

/// Implement `Display` trait for `ShadowFile` struct
impl std::fmt::Display for ShadowFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut shadow_file = String::new();
        for line in &self.lines {
            shadow_file.push_str(&line.to_string());
            shadow_file.push('\n');
        }
        write!(f, "{shadow_file}")
    }
}

impl ShadowFile {
    /// Method parse whole shadow file and return `ShadowFile` struct
    pub fn from_stream(mut stream: impl io::Read) -> Result<Self, Error> {
        let mut lines: String = String::new();
        stream.read_to_string(&mut lines)?;

        let shadow_lines = lines
            .lines()
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            lines: shadow_lines,
        })
    }

    /// Method build `ShadowFile` struct from provided file path
    pub fn from_file(file_path: impl AsRef<Path>) -> Result<Self, Error> {
        let file = File::open(file_path)?;
        Self::from_stream(file)
    }

    /// Method set password for user with provided `username` or return error if user does not exist
    pub fn set_password(
        &mut self,
        username: &str,
        new_password: Option<String>,
        password_hash_type: PasswordHashType,
    ) -> Result<(), Error> {
        self.lines
            .iter_mut()
            .find(|line| line.username == username)
            .ok_or(Error::Io(io::Error::new(
                io::ErrorKind::Other,
                format!("User `{username}` does not exist"),
            )))?
            .set_password(new_password, password_hash_type)
    }

    /// Method check credentials for provided `username` and `password`
    pub fn check_credentials(&self, username: &str, password: Option<&str>) -> bool {
        self.lines
            .iter()
            .find(|line| line.username == username)
            .is_some_and(|line| line.check_credentials(password))
    }
}

impl ShadowLine {
    /// Method change password in shadow line
    pub fn set_password(
        &mut self,
        new_password: Option<String>,
        hash_type: PasswordHashType,
    ) -> Result<(), Error> {
        // If new password is None, set password to None, otherwise hash new password.
        // Regardless of the old hash salt, we will use new salt for new password.
        // This means that new hash will differ from the old one in case of same password,
        // but it will increase security.
        self.password_hash = match new_password {
            None => None,
            Some(new_password) => Some(match hash_type {
                // OpenWRT platform uses md5_crypt
                PasswordHashType::Md5 =>
                {
                    #[expect(deprecated)]
                    md5_crypt::hash(new_password)?
                }
                PasswordHashType::Sha512 => sha512_crypt::hash(new_password)?,
            }),
        };
        Ok(())
    }

    /// Method check credentials for provided `username` and `password`
    pub fn check_credentials(&self, password: Option<&str>) -> bool {
        match (password, self.password_hash.as_ref()) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(password), Some(current_password_hash)) => {
                unix::verify(password, current_password_hash)
            }
        }
    }
}

/// Implement `Display` `ShadowLine` struct
impl std::fmt::Display for ShadowLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}:{}:{}:{}:{}:{}:",
            self.username,
            self.password_hash.as_ref().map_or("", |x| x.as_str()),
            self.last_change.map(|x| x.to_string()).unwrap_or_default(),
            self.min_days.map(|x| x.to_string()).unwrap_or_default(),
            self.max_days.map(|x| x.to_string()).unwrap_or_default(),
            self.warn_days.map(|x| x.to_string()).unwrap_or_default(),
            self.inactive_days
                .map(|x| x.to_string())
                .unwrap_or_default(),
            self.expire_days.map(|x| x.to_string()).unwrap_or_default(),
        )
    }
}

/// Implement `FromStr` for `ShadowLine` struct
impl FromStr for ShadowLine {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let split_result = s.split(':');
        let splits: Vec<&str> = split_result.collect();

        if splits.len() < 9 {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::Other,
                "Wrong format of shadow file line",
            )));
        }

        Ok(ShadowLine {
            username: splits[0].to_owned(),
            password_hash: Some(splits[1].to_owned()).filter(|s| !s.is_empty()),
            last_change: splits[2].parse::<u32>().ok(),
            min_days: splits[3].parse::<u32>().ok(),
            max_days: splits[4].parse::<u32>().ok(),
            warn_days: splits[5].parse::<u32>().ok(),
            inactive_days: splits[6].parse::<u32>().ok(),
            expire_days: splits[7].parse::<u32>().ok(),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const VALID_PASSWORD: &str = "test";
    const EMPTY_PASSWORD: &str = "";
    const WRONG_PASSWORD: &str = "wrong_password";
    const BLANK_PASSWORD: &str = " ";

    const TEST_SHADOW_FILE: &[u8] = b"user:$6$testsalt$tJbUl1kXqW33QAR3uSZ526jhi2VR/8b5Oc.fgGcuj1amRP1gtYnGoqbDwnND9jnHaR.tZ1.Uag0nWYDafTUxX0:18901:0:99999:7:::\n\
    without_pass::18901:0:99999:7:::\n\
    root::18901:0:99999:7:::\n\
    blank_pass:$6$testsalt$XqLHtTgHj/aLJzohoCs/2MPSyJNBDX5O/JHt0wbtRyZQglcZLEazOWjt2fNwqwFjqg3eNkiZaeDRCVkT/pwW..:18901:0:99999:7:::\n\
    ";

    #[test]
    fn user_does_not_exist_test() {
        let shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        assert!(
            !shadow_file.check_credentials("non_exist_user", Some(VALID_PASSWORD)),
            "BUG: Wrongly authorized user which does not exist"
        );
    }

    #[test]
    fn credentials_test() {
        let username = "user";
        let shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        // Right password test
        assert!(
            shadow_file.check_credentials(username, Some(VALID_PASSWORD)),
            "BUG: Failed authentication with valid credentials"
        );
        // Wrong password test
        assert!(
            !shadow_file.check_credentials(username, Some(WRONG_PASSWORD)),
            "BUG: Wrongly authorized user with wrong password"
        );
    }

    #[test]
    fn no_password_test() {
        let username = "without_pass";

        let shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        // User without password is authenticated with any password
        assert!(
            shadow_file.check_credentials(username, Some(EMPTY_PASSWORD)),
            "BUG: Failed authentication with valid credentials"
        );
        assert!(
            shadow_file.check_credentials(username, Some(BLANK_PASSWORD)),
            "BUG: Failed authentication with valid credentials"
        );
        assert!(
            shadow_file.check_credentials(username, Some(WRONG_PASSWORD)),
            "BUG: Failed authentication with valid credentials"
        );
        assert!(
            shadow_file.check_credentials(username, None),
            "BUG: Failed authentication with valid credentials"
        );
    }

    #[test]
    fn blank_password_test() {
        let username = "blank_pass";
        let shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        assert!(shadow_file.check_credentials(username, Some(BLANK_PASSWORD)));
        assert!(!shadow_file.check_credentials(username, Some(EMPTY_PASSWORD)));
        assert!(!shadow_file.check_credentials(username, Some(WRONG_PASSWORD)));
        assert!(!shadow_file.check_credentials(username, Some(VALID_PASSWORD)));
        assert!(!shadow_file.check_credentials(username, None));
    }

    #[test]
    fn matching_username_prefix_test() {
        let shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        let valid_username = "root";
        let invalid_usernames = ["", "r", "ro", "roo"];

        assert!(shadow_file.check_credentials(valid_username, Some(VALID_PASSWORD)));

        for invalid_username in invalid_usernames {
            assert!(!shadow_file.check_credentials(invalid_username, Some(VALID_PASSWORD)));
        }
    }

    // Test of serialize and deserialize methods
    #[tokio::test]
    async fn serialize_deserialize_test() {
        let shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        assert_eq!(
            TEST_SHADOW_FILE,
            shadow_file.to_string().as_bytes(),
            "BUG: Wrongly parsed shadow file"
        );
    }

    /// Test change password for user which does not exist
    #[tokio::test]
    async fn user_dont_exist() {
        let result = ShadowFile::from_stream(TEST_SHADOW_FILE)
            .expect("BUG: Failed to parse shadow file")
            .set_password(
                "user_dont_exist",
                Some("test".to_owned()),
                PasswordHashType::Sha512,
            );

        assert!(result.is_err());
        assert!(
            result
                .expect_err("BUG: impossible")
                .to_string()
                .contains("User `user_dont_exist` does not exist")
        );
    }

    /// Test change of password and authenticate
    #[tokio::test]
    async fn change_pass_value_and_authenticate() {
        let username = "root";
        let passwords = ["test1", "test2"];
        let password_type = PasswordHashType::Sha512;

        let mut shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        for password in passwords {
            shadow_file
                .set_password(username, Some(password.to_owned()), password_type)
                .expect("BUG: Failed to change password");

            assert!(
                shadow_file.check_credentials(username, Some(password)),
                "BUG: Failed to authenticate with new password"
            );
        }
    }

    /// Test password verification for md5 and sha256
    #[tokio::test]
    async fn change_pass_type_and_authenticate() {
        let username = "root";
        let password = "test";
        let password_types = [PasswordHashType::Md5, PasswordHashType::Sha512];

        let mut shadow_file =
            ShadowFile::from_stream(TEST_SHADOW_FILE).expect("BUG: Failed to parse shadow file");

        for password_type in password_types {
            shadow_file
                .set_password(username, Some(password.to_owned()), password_type)
                .expect("BUG: Failed to change password");

            assert!(
                shadow_file.check_credentials(username, Some(password)),
                "BUG: failed to verify a correct password"
            );
        }
    }
}
