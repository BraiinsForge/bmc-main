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
    const UPGRADE_RESULT_FILE: &'static str = "etc/upgrade_result";

    pub fn new(runtime_dir: impl AsRef<Path>) -> Self {
        Self {
            mockfs_root: runtime_dir.as_ref().to_owned(),
        }
    }

    pub fn init(&self) -> io::Result<()> {
        let src: &Path = self.mockfs_root.as_ref();

        let dirs = vec!["etc", "tmp"];
        for dir in dirs {
            fs::create_dir_all(src.join(dir))?;
        }
        Ok(())
    }

    #[must_use]
    pub fn upgrade_result(&self) -> PathBuf {
        self.build_mockfs_path(Self::UPGRADE_RESULT_FILE)
    }

    fn build_mockfs_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let stripped = match path
            .as_ref()
            .strip_prefix(std::path::MAIN_SEPARATOR.to_string())
        {
            Ok(p) => p,
            Err(_e) => path.as_ref(),
        };
        self.mockfs_root.join(stripped)
    }
}
