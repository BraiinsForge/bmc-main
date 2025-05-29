// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::Path;

use tokio::fs;

pub async fn read_to_string(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .await
        .ok()
        .map(|value| value.trim().to_owned())
}
