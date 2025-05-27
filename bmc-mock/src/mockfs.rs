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
    const FACTORY_DEFAULT_FILE: &'static str = "etc/factory-default";

    pub fn new(runtime_dir: impl AsRef<Path>) -> Self {
        Self {
            mockfs_root: runtime_dir.as_ref().to_owned(),
        }
    }

    pub fn init(&self, factory_default: bool) -> io::Result<()> {
        let src: &Path = self.mockfs_root.as_ref();

        let dirs = vec!["etc", "tmp"];
        for dir in dirs {
            fs::create_dir_all(src.join(dir))?;
        }
        self.add_or_remove_factory_default_flag(factory_default)?;
        Ok(())
    }

    #[must_use]
    pub fn upgrade_result(&self) -> PathBuf {
        self.build_mockfs_path(Self::UPGRADE_RESULT_FILE)
    }

    #[must_use]
    pub fn factory_default(&self) -> PathBuf {
        self.build_mockfs_path(Self::FACTORY_DEFAULT_FILE)
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

    fn add_or_remove_factory_default_flag(&self, add: bool) -> io::Result<()> {
        let path = self.factory_default();
        if add {
            fs::File::create(path)?;
        } else {
            _ = fs::remove_file(path);
        }

        Ok(())
    }
}
