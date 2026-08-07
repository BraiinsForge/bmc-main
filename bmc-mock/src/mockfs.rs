// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct MockFs {
    pub mockfs_root: PathBuf,
    template_dir: PathBuf,
}

impl MockFs {
    const UPGRADE_RESULT_FILE: &str = "etc/upgrade_result";
    const UPGRADE_SCENARIO_FILE: &str = "etc/upgrade-scenario.json";
    const FACTORY_DEFAULT_FILE: &str = "etc/factory-default";
    const DEVICE_SETUP_PENDING_FILE: &str = "etc/setup-pending";
    const WIFI_RECONFIG_FILE: &str = "etc/wifi-reconfig";
    const WIFI_STATUS_FILE: &str = "etc/is_wifi_enabled";
    const PENDING_INSTALL_FILE: &str = "dev/shm/bmc-nix-pending-install.json";
    const SERVICE_UPGRADE_MARKER_FILE: &str = "dev/shm/bmc-service-upgraded/bmc-compositor";

    pub fn new(template_dir: impl AsRef<Path>, runtime_dir: impl AsRef<Path>) -> Self {
        Self {
            mockfs_root: runtime_dir.as_ref().to_owned(),
            template_dir: template_dir.as_ref().to_owned(),
        }
    }

    pub fn init(&self, reset: bool, factory_default: bool, setup_pending: bool) -> io::Result<()> {
        let src: &Path = self.mockfs_root.as_ref();

        let dirs = vec!["etc", "tmp", "dev/shm"];
        for dir in dirs {
            fs::create_dir_all(src.join(dir))?;
        }

        copy_recursive(&self.template_dir, &self.mockfs_root, reset)?;

        self.add_or_remove_flag(factory_default, &self.factory_default())?;
        self.add_or_remove_flag(setup_pending, &self.setup_pending())?;

        Ok(())
    }

    #[must_use]
    pub fn upgrade_result(&self) -> PathBuf {
        self.build_mockfs_path(Self::UPGRADE_RESULT_FILE)
    }

    #[must_use]
    pub fn upgrade_scenario(&self) -> PathBuf {
        self.build_mockfs_path(Self::UPGRADE_SCENARIO_FILE)
    }

    #[must_use]
    pub fn pending_install(&self) -> PathBuf {
        self.build_mockfs_path(Self::PENDING_INSTALL_FILE)
    }

    #[must_use]
    pub fn service_upgrade_marker(&self) -> PathBuf {
        self.build_mockfs_path(Self::SERVICE_UPGRADE_MARKER_FILE)
    }

    #[must_use]
    pub fn factory_default(&self) -> PathBuf {
        self.build_mockfs_path(Self::FACTORY_DEFAULT_FILE)
    }

    #[must_use]
    pub fn setup_pending(&self) -> PathBuf {
        self.build_mockfs_path(Self::DEVICE_SETUP_PENDING_FILE)
    }

    #[must_use]
    pub fn wifi_reconfig(&self) -> PathBuf {
        self.build_mockfs_path(Self::WIFI_RECONFIG_FILE)
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

    pub fn add_or_remove_flag(&self, add: bool, path: &PathBuf) -> io::Result<()> {
        if add {
            fs::File::create(path)?;
        } else {
            _ = fs::remove_file(path);
        }

        Ok(())
    }

    #[must_use]
    pub fn is_wifi_enabled(&self) -> PathBuf {
        self.build_mockfs_path(Self::WIFI_STATUS_FILE)
    }
}

fn copy_recursive(src: impl AsRef<Path>, dst: impl AsRef<Path>, overwrite: bool) -> io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;

        let from = entry.path();
        let to = dst.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_recursive(from, to, overwrite)?;
        } else if !to.exists() || overwrite {
            fs::copy(from, to)?;
        }
    }
    Ok(())
}
