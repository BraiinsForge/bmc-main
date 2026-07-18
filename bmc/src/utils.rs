// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;
use tokio::io::AsyncWriteExt as _;

const TMP_FILE_EXTENSION: &str = "tmp";

pub async fn read_to_string(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .await
        .ok()
        .map(|value| value.trim().to_owned())
}

pub async fn get_modified_from_path(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .await
        .and_then(|metadata| metadata.modified())
        .ok()
}

pub async fn replace_file(
    path: impl AsRef<Path>,
    data: &[u8],
) -> Result<Option<SystemTime>, std::io::Error> {
    // Both the temporary file and the final rename target live in the
    // destination's parent directory, so it must exist first. On a
    // fresh install the config lives under a directory (`/etc/bmc/`)
    // that nothing else creates, so without this every save would fail
    // with ENOENT and settings would silently never persist.
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent).await?;
    }

    // at first, store data into temporary file and then move it to the final path
    // this prevents interference with other external processes
    let mut tmp_path = PathBuf::from(path.as_ref());
    tmp_path.set_extension(TMP_FILE_EXTENSION);
    {
        let mut file = fs::File::create(&tmp_path).await?;
        file.write_all(data).await?;
    }
    let modified = get_modified_from_path(&tmp_path).await;
    fs::rename(&tmp_path, path).await?;
    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replace_file_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        // A parent directory that does not exist yet, mirroring a
        // fresh install where `/etc/bmc/` is not on disk.
        let path = dir.path().join("bmc").join("config.json");
        replace_file(&path, b"payload")
            .await
            .expect("BUG: replace_file must create parents and write");
        let written = fs::read_to_string(&path).await.expect("BUG: read back");
        assert_eq!(written, "payload");
    }
}
