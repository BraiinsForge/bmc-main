// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{ffi::OsStr, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

#[derive(Debug)]
pub struct Audio;

impl Audio {
    /// Plays a sound from the given file path.
    ///
    /// # Arguments
    ///
    /// * `file_path` - A string slice that holds the path to the audio file in wav format.
    pub async fn play<S: AsRef<OsStr>>(
        file_path: S,
        cancellation_token: CancellationToken,
    ) -> anyhow::Result<()> {
        // Run the command to play the audio file using the `aplay` command line utility in async mode.
        use tokio::process::Command;
        let mut child = Command::new("aplay")
            .arg(file_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn aplay: {}", e))?;

        tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) if status.success() => Ok(()),
                    Ok(status) => Err(anyhow::anyhow!("Failed to play audio: {}", status)),
                    Err(e) => Err(anyhow::anyhow!("Failed to wait on aplay: {}", e)),
                }
            },
            ()  = cancellation_token.cancelled() => {
                if let Err(e) = child.kill().await {
                    error!("Warning: failed to kill aplay: {}", e);
                }

                let _ = child.wait().await;
                Ok(())
            }
        }
    }

    pub async fn set_volume(value: u8) -> anyhow::Result<()> {
        //todo
        tokio::time::sleep(Duration::from_millis(100)).await;
        info!("Setting volume to {value}%");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::Audio;

    // Ignore this test if the audio file does not exist.
    #[ignore]
    #[tokio::test]
    async fn test_play_audio() {
        // This test will only run if an audio file exists at the specified path.
        // Adjust the path to a valid audio file for your environment.
        let file_path = "/root/test.wav";
        let token = CancellationToken::new();
        Audio::play(file_path, token)
            .await
            .expect("BUG: Failed test playing audio");
    }
}
