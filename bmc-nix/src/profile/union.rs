// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use crate::profile::BuildProfileError;
use crate::types::ResolvedPackage;

#[derive(Clone)]
struct Provider<'a> {
    package: &'a ResolvedPackage,
    source_path: PathBuf,
    kind: ProviderKind,
    ancestors: Vec<(u64, u64)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderKind {
    Leaf,
    Directory,
}

enum UnionNode<'a> {
    Leaf(Provider<'a>),
    LinkableDirectory(Provider<'a>),
    MergedDirectory(Vec<Provider<'a>>),
}

pub(super) fn build_symlink_tree(
    tmp_path: &Path,
    packages: &[ResolvedPackage],
) -> Result<(), BuildProfileError> {
    let providers = packages
        .iter()
        .map(root_provider)
        .collect::<Result<Vec<_>, _>>()?;

    materialize_merged_directory(tmp_path, Path::new(""), &providers)
}

fn root_provider(package: &ResolvedPackage) -> Result<Provider<'_>, BuildProfileError> {
    let source_path = PathBuf::from(&package.store_path);
    let (kind, identity) = classify_source_path(&source_path)?;
    let ancestors = identity.into_iter().collect();

    Ok(Provider {
        package,
        source_path,
        kind,
        ancestors,
    })
}

fn child_provider<'a>(
    parent: &Provider<'a>,
    child_name: &Path,
) -> Result<Provider<'a>, BuildProfileError> {
    let source_path = parent.source_path.join(child_name);
    let (kind, _identity) = classify_source_path(&source_path)?;

    Ok(Provider {
        package: parent.package,
        source_path,
        kind,
        ancestors: parent.ancestors.clone(),
    })
}

fn classify_source_path(
    path: &Path,
) -> Result<(ProviderKind, Option<(u64, u64)>), BuildProfileError> {
    let lstat =
        std::fs::symlink_metadata(path).map_err(|source| BuildProfileError::StatStorePath {
            path: path.display().to_string(),
            source,
        })?;

    if lstat.is_symlink() {
        return match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => Ok((
                ProviderKind::Directory,
                Some((metadata.dev(), metadata.ino())),
            )),
            Ok(_metadata) => Ok((ProviderKind::Leaf, None)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok((ProviderKind::Leaf, None))
            }
            Err(source) => Err(BuildProfileError::StatStorePath {
                path: path.display().to_string(),
                source,
            }),
        };
    }

    if lstat.is_dir() {
        Ok((ProviderKind::Directory, Some((lstat.dev(), lstat.ino()))))
    } else {
        Ok((ProviderKind::Leaf, None))
    }
}

fn resolve_node<'a>(
    rel_path: &Path,
    providers: Vec<Provider<'a>>,
) -> Result<UnionNode<'a>, BuildProfileError> {
    let leaf_count = providers
        .iter()
        .filter(|provider| provider.kind == ProviderKind::Leaf)
        .count();
    let directory_count = providers.len() - leaf_count;

    match (leaf_count, directory_count) {
        (1, 0) => Ok(UnionNode::Leaf(
            providers
                .into_iter()
                .next()
                .expect("BUG: one provider should exist"),
        )),
        (0, 1) => Ok(UnionNode::LinkableDirectory(
            providers
                .into_iter()
                .next()
                .expect("BUG: one provider should exist"),
        )),
        (0, _) => Ok(UnionNode::MergedDirectory(providers)),
        _ => conflict(rel_path, &providers),
    }
}

fn conflict<'a>(
    rel_path: &Path,
    providers: &[Provider<'a>],
) -> Result<UnionNode<'a>, BuildProfileError> {
    let (provider_a, remaining) = providers
        .split_first()
        .expect("BUG: conflict requires at least one provider");
    let provider_b = if provider_a.kind == ProviderKind::Leaf {
        remaining
            .first()
            .expect("BUG: leaf conflict requires a second provider")
    } else {
        remaining
            .iter()
            .find(|provider| provider.kind == ProviderKind::Leaf)
            .expect("BUG: mixed conflict requires a leaf provider")
    };

    Err(BuildProfileError::Conflict {
        path: rel_path.display().to_string(),
        pkg_a: provider_a.package.name.clone(),
        pkg_b: provider_b.package.name.clone(),
    })
}

fn materialize_node(
    tmp_path: &Path,
    rel_path: &Path,
    node: UnionNode<'_>,
) -> Result<(), BuildProfileError> {
    match node {
        UnionNode::Leaf(provider) | UnionNode::LinkableDirectory(provider) => {
            create_symlink(&provider.source_path, &tmp_path.join(rel_path))
        }
        UnionNode::MergedDirectory(mut providers) => {
            for provider in &mut providers {
                let identity = directory_identity(&provider.source_path)?;
                if provider.ancestors.contains(&identity) {
                    return Err(BuildProfileError::SymlinkCycle {
                        path: provider.source_path.display().to_string(),
                    });
                }
                provider.ancestors.push(identity);
            }

            let dst_path = tmp_path.join(rel_path);
            std::fs::create_dir_all(&dst_path).map_err(|source| BuildProfileError::CreateDir {
                path: dst_path.display().to_string(),
                source,
            })?;

            materialize_merged_directory(tmp_path, rel_path, &providers)
        }
    }
}

fn materialize_merged_directory(
    tmp_path: &Path,
    rel_path: &Path,
    providers: &[Provider<'_>],
) -> Result<(), BuildProfileError> {
    let mut children: BTreeMap<PathBuf, Vec<Provider<'_>>> = BTreeMap::new();

    for provider in providers {
        for entry in read_dir_sorted(&provider.source_path)? {
            let child_name = PathBuf::from(entry.file_name());
            let child = child_provider(provider, &child_name)?;
            children.entry(child_name).or_default().push(child);
        }
    }

    for (child_name, child_providers) in children {
        let child_rel_path = rel_path.join(&child_name);
        let node = resolve_node(&child_rel_path, child_providers)?;
        materialize_node(tmp_path, &child_rel_path, node)?;
    }

    Ok(())
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<std::fs::DirEntry>, BuildProfileError> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|source| BuildProfileError::ReadStorePath {
            path: dir.display().to_string(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| BuildProfileError::ReadStorePath {
            path: dir.display().to_string(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    Ok(entries)
}

fn directory_identity(path: &Path) -> Result<(u64, u64), BuildProfileError> {
    let metadata = std::fs::metadata(path).map_err(|source| BuildProfileError::StatStorePath {
        path: path.display().to_string(),
        source,
    })?;

    Ok((metadata.dev(), metadata.ino()))
}

fn create_symlink(source_path: &Path, dst_path: &Path) -> Result<(), BuildProfileError> {
    std::os::unix::fs::symlink(source_path, dst_path).map_err(|source| {
        BuildProfileError::CreateSymlink {
            path: dst_path.display().to_string(),
            source,
        }
    })
}
