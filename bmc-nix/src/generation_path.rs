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

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum GenerationPathError {
    #[error("invalid relative path `{path}`")]
    InvalidRelativePath { path: String },
    #[error("failed to create directory `{path}`: {source}")]
    CreateDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to stat `{path}`: {source}")]
    Stat {
        path: String,
        source: std::io::Error,
    },
    #[error("path is not a directory `{path}`")]
    NotDirectory { path: String },
    #[error("failed to read symlink `{path}`: {source}")]
    ReadLink {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to read directory `{path}`: {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to remove `{path}`: {source}")]
    Remove {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to create symlink `{path}`: {source}")]
    CreateSymlink {
        path: String,
        source: std::io::Error,
    },
    #[error("target is a directory `{path}`")]
    TargetIsDirectory { path: String },
    #[error("failed to write `{path}`: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to set permissions `{path}`: {source}")]
    SetPermissions {
        path: String,
        source: std::io::Error,
    },
}

pub fn ensure_directory(
    generation_root: &Path,
    relative_directory: &Path,
) -> Result<(), GenerationPathError> {
    let components = validate_relative_path(relative_directory)?;
    let mut current = PathBuf::from(generation_root);

    for component in components {
        current.push(component);
        ensure_component_directory(&current)?;
    }

    Ok(())
}

pub fn write_generated_file(
    generation_root: &Path,
    relative_file: &Path,
    contents: &[u8],
    mode: u32,
) -> Result<(), GenerationPathError> {
    let components = validate_relative_path(relative_file)?;
    let Some((_file_name, parent_components)) = components.split_last() else {
        return Err(GenerationPathError::InvalidRelativePath {
            path: relative_file.display().to_string(),
        });
    };

    let parent = parent_components.iter().collect::<PathBuf>();
    ensure_directory(generation_root, &parent)?;

    let target = generation_root.join(relative_file);
    prepare_file_target(&target)?;

    fs::write(&target, contents).map_err(|source| GenerationPathError::Write {
        path: target.display().to_string(),
        source,
    })?;

    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(&target, permissions).map_err(|source| {
        GenerationPathError::SetPermissions {
            path: target.display().to_string(),
            source,
        }
    })?;

    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<Vec<OsString>, GenerationPathError> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => components.push(component.to_os_string()),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(GenerationPathError::InvalidRelativePath {
                    path: path.display().to_string(),
                });
            }
        }
    }

    Ok(components)
}

fn ensure_component_directory(path: &Path) -> Result<(), GenerationPathError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => materialize_symlinked_directory(path),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_metadata) => Err(GenerationPathError::NotDirectory {
            path: path.display().to_string(),
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| GenerationPathError::CreateDir {
                path: path.display().to_string(),
                source,
            })
        }
        Err(source) => Err(GenerationPathError::Stat {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn materialize_symlinked_directory(path: &Path) -> Result<(), GenerationPathError> {
    let link_target = fs::read_link(path).map_err(|source| GenerationPathError::ReadLink {
        path: path.display().to_string(),
        source,
    })?;
    let target_dir = resolve_symlink_target(path, &link_target);

    let target_metadata =
        fs::metadata(&target_dir).map_err(|source| GenerationPathError::Stat {
            path: target_dir.display().to_string(),
            source,
        })?;
    if !target_metadata.is_dir() {
        return Err(GenerationPathError::NotDirectory {
            path: path.display().to_string(),
        });
    }

    let child_names = read_child_names(&target_dir)?;

    fs::remove_file(path).map_err(|source| GenerationPathError::Remove {
        path: path.display().to_string(),
        source,
    })?;
    fs::create_dir(path).map_err(|source| GenerationPathError::CreateDir {
        path: path.display().to_string(),
        source,
    })?;

    for child_name in child_names {
        let child_path = path.join(&child_name);
        let child_target = target_dir.join(child_name);
        symlink(&child_target, &child_path).map_err(|source| {
            GenerationPathError::CreateSymlink {
                path: child_path.display().to_string(),
                source,
            }
        })?;
    }

    Ok(())
}

fn resolve_symlink_target(link_path: &Path, link_target: &Path) -> PathBuf {
    if link_target.is_absolute() {
        link_target.to_path_buf()
    } else {
        link_path
            .parent()
            .expect("BUG: component path should have a parent")
            .join(link_target)
    }
}

fn read_child_names(path: &Path) -> Result<Vec<OsString>, GenerationPathError> {
    let entries = fs::read_dir(path).map_err(|source| GenerationPathError::ReadDir {
        path: path.display().to_string(),
        source,
    })?;

    let mut child_names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| GenerationPathError::ReadDir {
            path: path.display().to_string(),
            source,
        })?;
        child_names.push(entry.file_name());
    }
    child_names.sort();

    Ok(child_names)
}

fn prepare_file_target(path: &Path) -> Result<(), GenerationPathError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => prepare_symlink_file_target(path),
        Ok(metadata) if metadata.is_dir() => Err(GenerationPathError::TargetIsDirectory {
            path: path.display().to_string(),
        }),
        Ok(_metadata) => fs::remove_file(path).map_err(|source| GenerationPathError::Remove {
            path: path.display().to_string(),
            source,
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(GenerationPathError::Stat {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn prepare_symlink_file_target(path: &Path) -> Result<(), GenerationPathError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Err(GenerationPathError::TargetIsDirectory {
            path: path.display().to_string(),
        }),
        Ok(_metadata) => fs::remove_file(path).map_err(|source| GenerationPathError::Remove {
            path: path.display().to_string(),
            source,
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => {
            fs::remove_file(path).map_err(|source| GenerationPathError::Remove {
                path: path.display().to_string(),
                source,
            })
        }
        Err(source) => Err(GenerationPathError::Stat {
            path: path.display().to_string(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt as _;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use super::{GenerationPathError, ensure_directory, write_generated_file};

    #[test]
    fn ensure_directory_materializes_every_symlinked_ancestor() {
        let tmp = tempfile::tempdir().expect("BUG: create temp dir");
        let generation = tmp.path().join("generation");
        let store = tmp.path().join("store");

        std::fs::create_dir_all(store.join("core/activation/scripts"))
            .expect("BUG: create store scripts dir");
        std::fs::write(
            store.join("core/activation/scripts/10-existing"),
            "existing",
        )
        .expect("BUG: write store script");
        std::fs::create_dir_all(&generation).expect("BUG: create generation dir");
        symlink(store.join("core"), generation.join("core")).expect("BUG: create core symlink");

        ensure_directory(&generation, Path::new("core/activation/scripts"))
            .expect("BUG: materialize scripts dir");

        assert_real_directory(&generation.join("core"));
        assert_real_directory(&generation.join("core/activation"));
        assert_real_directory(&generation.join("core/activation/scripts"));

        let generated_script = generation.join("core/activation/scripts/10-existing");
        let metadata =
            std::fs::symlink_metadata(&generated_script).expect("BUG: stat generated script");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            std::fs::read_link(&generated_script).expect("BUG: read generated script symlink"),
            store.join("core/activation/scripts/10-existing")
        );
    }

    #[test]
    fn write_generated_file_replaces_symlink_without_mutating_store() {
        let tmp = tempfile::tempdir().expect("BUG: create temp dir");
        let generation = tmp.path().join("generation");
        let store = tmp.path().join("store");

        std::fs::create_dir_all(store.join("core/activation"))
            .expect("BUG: create store activation dir");
        std::fs::write(store.join("core/activation/entrypoint"), "old entrypoint")
            .expect("BUG: write store entrypoint");
        std::fs::create_dir_all(&generation).expect("BUG: create generation dir");
        symlink(store.join("core"), generation.join("core")).expect("BUG: create core symlink");

        write_generated_file(
            &generation,
            Path::new("core/activation/entrypoint"),
            b"generated entrypoint",
            0o755,
        )
        .expect("BUG: write generated entrypoint");

        let generated = generation.join("core/activation/entrypoint");
        let metadata = std::fs::symlink_metadata(&generated).expect("BUG: stat generated file");
        assert!(metadata.is_file());
        assert!(!metadata.file_type().is_symlink());
        assert_eq!(metadata.mode() & 0o777, 0o755);
        assert_eq!(
            std::fs::read_to_string(&generated).expect("BUG: read generated file"),
            "generated entrypoint"
        );
        assert_eq!(
            std::fs::read_to_string(store.join("core/activation/entrypoint"))
                .expect("BUG: read store entrypoint"),
            "old entrypoint"
        );
    }

    #[test]
    fn write_generated_file_rejects_existing_directory_target() {
        let tmp = tempfile::tempdir().expect("BUG: create temp dir");
        let generation = tmp.path().join("generation");
        std::fs::create_dir_all(generation.join("core/activation/entrypoint"))
            .expect("BUG: create existing target dir");

        let err = write_generated_file(
            &generation,
            Path::new("core/activation/entrypoint"),
            b"generated entrypoint",
            0o755,
        )
        .expect_err("BUG: directory target should fail");

        assert!(matches!(err, GenerationPathError::TargetIsDirectory { .. }));
    }

    #[test]
    fn write_generated_file_rejects_final_symlink_to_directory() {
        let tmp = tempfile::tempdir().expect("BUG: create temp dir");
        let generation = tmp.path().join("generation");
        let store = tmp.path().join("store");

        std::fs::create_dir_all(store.join("manifest")).expect("BUG: create store manifest dir");
        std::fs::create_dir_all(&generation).expect("BUG: create generation dir");
        symlink(store.join("manifest"), generation.join("manifest"))
            .expect("BUG: create manifest symlink");

        let err = write_generated_file(
            &generation,
            Path::new("manifest"),
            b"generated manifest",
            0o644,
        )
        .expect_err("BUG: symlink to directory target should fail");

        assert!(matches!(err, GenerationPathError::TargetIsDirectory { .. }));
        let metadata = std::fs::symlink_metadata(generation.join("manifest"))
            .expect("BUG: stat manifest symlink");
        assert!(metadata.file_type().is_symlink());
    }

    #[test]
    fn generation_paths_reject_absolute_or_parent_components() {
        let tmp = tempfile::tempdir().expect("BUG: create temp dir");
        let generation = tmp.path().join("generation");
        std::fs::create_dir_all(&generation).expect("BUG: create generation dir");

        for path in [
            Path::new("/absolute"),
            Path::new("../escape"),
            Path::new("a/../b"),
        ] {
            let err = ensure_directory(&generation, path)
                .expect_err("BUG: invalid ensure_directory path should fail");
            assert!(matches!(
                err,
                GenerationPathError::InvalidRelativePath { .. }
            ));

            let err = write_generated_file(&generation, path, b"contents", 0o644)
                .expect_err("BUG: invalid write_generated_file path should fail");
            assert!(matches!(
                err,
                GenerationPathError::InvalidRelativePath { .. }
            ));
        }
    }

    fn assert_real_directory(path: &Path) {
        let metadata = std::fs::symlink_metadata(path).expect("BUG: stat materialized dir");
        assert!(metadata.is_dir());
        assert!(!metadata.file_type().is_symlink());
    }
}
