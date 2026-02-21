// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::Path;

use anyhow::{Result, anyhow};
use sysinfo::Disks;

pub(crate) struct StorageChecker;

impl StorageChecker {
    /// Gets available disk size in bytes for a given path.
    ///
    /// It iterates through path ancestors and checks if the path is a mount point of a disk.
    /// E.g. for a path "/tmp/firmware.tar" it iterates through [`/tmp/firmware.tar`, `/tmp`, `/`] and checks if the path is a mount point
    /// of a disk. If no disk is found for a given path it returns an error. In normal case there should be a path "/". Otherwise available
    /// space of the disk is returned.
    fn available_disk_space(path: &Path) -> Result<u64> {
        let path_ancestors = path.ancestors();

        let disks = Disks::new_with_refreshed_list();
        let disks = disks.list();
        let disk = path_ancestors
            .into_iter()
            .find_map(|path| disks.iter().find(|disk| disk.mount_point() == path))
            .ok_or(anyhow!("Failed to check disk space"))?;

        Ok(disk.available_space())
    }

    pub(crate) fn check_disk_space(path: &Path, required_size: u64) -> Result<()> {
        let available_space = Self::available_disk_space(path)?;

        if available_space < required_size {
            return Err(anyhow!(
                "Not enough space in path: {path:?}. Required: {required_size} bytes, got: {available_space} bytes"
            ));
        }

        Ok(())
    }
}
