// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{ffi::OsStr, fmt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Volume(u8);

impl Volume {
    pub fn new(value: u8) -> Result<Self, VolumeError> {
        if value > Volume::MAX.0 {
            Err(VolumeError::OutOfRange { value })
        } else {
            Ok(Volume(value))
        }
    }

    /// Returns decibels in range -40dB to 0dB (100 volume = 0dB).
    #[must_use]
    #[expect(clippy::cast_possible_truncation)]
    pub fn to_decibels(self) -> i8 {
        // Accurate mapping using float to avoid integer division warning
        let db = f32::from(self.0) * 0.4 - 40.0;
        db.round() as i8
    }

    pub const MAX: Volume = Volume(100);
    pub const MIN: Volume = Volume(0);
}

impl TryFrom<u8> for Volume {
    type Error = VolumeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Volume> for u8 {
    fn from(volume: Volume) -> Self {
        volume.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeError {
    OutOfRange { value: u8 },
}

impl fmt::Display for VolumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VolumeError::OutOfRange { value } => {
                write!(f, "Volume value {value} is out of range (0-100)")
            }
        }
    }
}

impl std::error::Error for VolumeError {}

#[derive(Debug, Clone, Copy)]
pub struct Audio {
    volume: Volume,
}

impl Audio {
    #[must_use]
    pub fn new() -> Self {
        Self {
            volume: Volume::new(50).expect("BUG: default volume 50 is within valid range"),
        }
    }

    pub async fn play<S: AsRef<OsStr>>(
        &self,
        file_path: S,
        cancellation_token: CancellationToken,
    ) -> anyhow::Result<()> {
        let mut child = Command::new("madplay")
            .arg(file_path)
            .arg("-A")
            .arg(self.volume.to_decibels().to_string())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn madplay: {}", e))?;

        tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) if status.success() => Ok(()),
                    Ok(status) => Err(anyhow::anyhow!("Failed to play audio: {}", status)),
                    Err(e) => Err(anyhow::anyhow!("Failed to wait on madplay: {}", e)),
                }
            },
            ()  = cancellation_token.cancelled() => {
                if let Err(e) = child.kill().await {
                    error!("Warning: failed to kill madplay: {}", e);
                }

                let _ = child.wait().await;
                Ok(())
            }
        }
    }

    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
    }

    #[must_use]
    pub fn volume(&self) -> Volume {
        self.volume
    }
}

#[cfg(test)]
mod tests {
    use super::{Audio, Volume};

    #[test]
    fn test_volume_and_audio() {
        // Test Volume validation and decibels conversion
        assert!(Volume::new(50).is_ok());
        assert!(Volume::new(101).is_err());
        assert_eq!(
            Volume::new(0)
                .expect("BUG: Failed to create volume 0")
                .to_decibels(),
            -40
        );
        assert_eq!(
            Volume::new(100)
                .expect("BUG: Failed to create volume 100")
                .to_decibels(),
            0
        );
        assert_eq!(
            Volume::new(50)
                .expect("BUG: Failed to create volume 50")
                .to_decibels(),
            -20
        );

        // Test Audio volume functionality
        let mut audio = Audio::new();
        assert_eq!(u8::from(audio.volume()), 50); // Default volume

        let new_volume = Volume::new(75).expect("BUG: Failed to create volume 75");
        audio.set_volume(new_volume);
        assert_eq!(u8::from(audio.volume()), 75);
        assert_eq!(audio.volume().to_decibels(), -10);
    }
}
