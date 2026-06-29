// Copyright (C) 2026  Braiins Systems s.r.o.

use std::ffi::OsStr;
use std::path::Path;

/// Pattern matching a path where leaf collisions between packages are tolerated.
///
/// See `ALLOWED_COLLISIONS` for the curated list. Match semantics:
///
/// - `FilePath(p)` matches the exact colliding path.
/// - `FileName(n)` matches the basename of the colliding path.
/// - `DirPath(p)` matches `p` itself or any descendant, segment-aware.
/// - `DirName(n)` matches any path component of the colliding path.
#[derive(Debug, Clone, Copy)]
enum CollisionRule {
    FilePath(&'static str),
    FileName(&'static str),
    DirPath(&'static str),
    DirName(&'static str),
}

/// Curated paths where leaf collisions between packages are tolerated.
///
/// On collision, the first package's symlink wins; subsequent collisions
/// matching a rule are dropped by the profile union builder.
const ALLOWED_COLLISIONS: &[CollisionRule] = &[
    CollisionRule::DirName("nix-support"),
    CollisionRule::DirPath("share/info"),
    CollisionRule::DirPath("share/mime"),
    CollisionRule::FilePath("share/applications/mimeinfo.cache"),
    CollisionRule::FileName("icon-theme.cache"),
    CollisionRule::FileName("index.theme"),
    CollisionRule::FilePath("share/glib-2.0/schemas/gschemas.compiled"),
    CollisionRule::DirPath("var/cache/fontconfig"),
    CollisionRule::FileName("fonts.dir"),
    CollisionRule::FileName("fonts.scale"),
    CollisionRule::DirPath("share/doc"),
    CollisionRule::DirPath("share/gtk-doc"),
    CollisionRule::DirPath("share/devhelp"),
    CollisionRule::DirPath("share/man"),
    CollisionRule::DirPath("share/locale"),
    CollisionRule::DirPath("share/gettext"),
    CollisionRule::FileName("perllocal.pod"),
    CollisionRule::FileName(".packlist"),
    CollisionRule::DirName("__pycache__"),
    CollisionRule::DirPath("share/aclocal"),
    CollisionRule::DirPath("share/libtool"),
    CollisionRule::DirPath("share/bash-completion"),
    CollisionRule::DirPath("etc/bash_completion.d"),
    CollisionRule::DirPath("share/zsh/site-functions"),
    CollisionRule::DirPath("share/zsh/vendor-completions"),
    CollisionRule::DirPath("share/fish/vendor_completions.d"),
    CollisionRule::DirPath("share/fish/vendor_conf.d"),
    CollisionRule::DirPath("share/fish/vendor_functions.d"),
    CollisionRule::DirPath("etc/profile.d"),
];

pub(super) fn allowed(rel_path: &Path) -> bool {
    ALLOWED_COLLISIONS
        .iter()
        .any(|rule| rule_matches(rel_path, *rule))
}

fn rule_matches(rel_path: &Path, rule: CollisionRule) -> bool {
    match rule {
        CollisionRule::FilePath(path) => rel_path == Path::new(path),
        CollisionRule::FileName(name) => rel_path.file_name() == Some(OsStr::new(name)),
        CollisionRule::DirPath(path) => rel_path.starts_with(Path::new(path)),
        CollisionRule::DirName(name) => rel_path
            .components()
            .any(|component| component.as_os_str() == OsStr::new(name)),
    }
}
