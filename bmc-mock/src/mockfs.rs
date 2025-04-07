// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct MockFs {
    pub mockfs_root: PathBuf,
}

impl MockFs {
    pub fn new(runtime_dir: impl AsRef<Path>) -> Self {
        Self {
            mockfs_root: runtime_dir.as_ref().to_owned(),
        }
    }

    pub fn init(&self) -> io::Result<()> {
        let src: &Path = self.mockfs_root.as_ref();
        fs::create_dir_all(src)
    }
}
