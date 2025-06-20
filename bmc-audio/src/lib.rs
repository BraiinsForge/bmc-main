// Copyright (C) 2025  Braiins Systems s.r.o.

#[derive(Debug)]
pub struct Audio;

impl Audio {
    /// Plays a sound from the given file path.
    ///
    /// # Arguments
    ///
    /// * `file_path` - A string slice that holds the path to the audio file in wav format.
    pub async fn play(file_path: &str) -> anyhow::Result<()> {
        // Run the command to play the audio file using the `aplay` command line utility in async mode.
        use tokio::process::Command;

        let status = Command::new("aplay").arg(file_path).status().await;

        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(anyhow::anyhow!("Failed to play audio: {}", status)),
            Err(e) => Err(anyhow::anyhow!("Failed to execute command: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Audio;

    // Ignore this test if the audio file does not exist.
    #[ignore]
    #[tokio::test]
    async fn test_play_audio() {
        // This test will only run if an audio file exists at the specified path.
        // Adjust the path to a valid audio file for your environment.
        let file_path = "/root/test.wav";
        Audio::play(file_path)
            .await
            .expect("BUG: Failed test playing audio");
    }
}
