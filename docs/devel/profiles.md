# BMC Profiles

BMC profiles are custom generation directories built from selected Nix store paths. They are not `nix profile` profiles.
The profile manager lives in `bmc-nix` and creates rollback-friendly generations under
`/nix/var/nix/gcroots/profiles/bmc`.

Each generation is a filesystem view of the selected packages plus generated profile metadata. Most entries are symlinks
into `/nix/store`; files generated during the profile build live directly in the generation directory.

## Generation Layout

A profile directory has numbered generation directories and a `current` symlink:

```text
/nix/var/nix/gcroots/profiles/bmc/
|-- 1-link/
|-- 2-link/
|-- current -> 2-link
`-- .lock
```

Each `N-link` directory is complete enough to activate or roll back independently. It contains package-provided paths
such as `bin/`, `lib/`, `etc/`, `core/activation/scripts/`, `hooks/`, `special/copy/`, plus generated files such as
`manifest` and `core/activation/entrypoint`.

## Build Flow

`bmc_nix::profile::build_profile` builds a new generation in a temporary directory, then renames it to `N-link` only
after the build succeeds:

1. Create `<N>-link.tmp` under the profile directory.
2. Build the package symlink tree from all resolved package store paths.
3. Run profile-build hooks from `hooks/` or from `--hooks-override-path`.
4. Write the generated `manifest`.
5. Rename `<N>-link.tmp` to `<N>-link`.

Activation is separate. Building a generation does not make it active and does not change `/etc`, services, or the
`current` symlink.

## Symlink Tree

The profile tree is optimized for a small number of symlinks:

- if exactly one package provides a directory at a relative path, the profile links that whole directory at the highest
  possible level;
- if multiple packages provide the same directory, the profile materializes that directory and resolves its children
  recursively;
- leaf entries, including regular files, file symlinks, and dangling symlinks, are symlinked into the store;
- a file-vs-file or file-vs-directory collision at the same relative path is a profile build conflict.

Do not assume a profile path component is a real directory. For example, `bin` might be a symlink when one package owns
all binaries, while `etc` might be a real directory when several packages contribute service files.

Code that reads from a generation should tolerate both real directories and directory symlinks. Code that writes
generated files into a generation must not blindly follow symlinked ancestors into the store.

## Package Contents

Packages are assembled by `nix/package.nix::mkPackage`. A package may contribute:

- a base derivation, whose output is included in the profile;
- `hooks/`, run during profile build;
- `core/activation/scripts/`, run during activation;
- `etc/init.d/`, `etc/rc.d/`, and `etc/init.d.conf/`, used by service orchestration;
- `special/copy/`, copied to the live root filesystem during activation;
- ordinary output files such as scripts under `bin/`.

Use package contents for static files that can remain symlinks into `/nix/store`. Use generated profile files only when
the profile builder or a built-in hook must synthesize content from the whole generation.

## Hooks

Hooks run during profile build, after the package symlink tree exists and before the generated manifest is written. They
receive:

```text
PROFILE_NEW_GENERATION=/path/to/<N>-link.tmp
```

Hooks do not receive the old generation and must not depend on `current` or the active system. This matters for factory
tarball builds, where hooks may run on the build host through `--hooks-override-path` while the target profile contains
ARM binaries.

Hooks may inspect the new generation and generate additional profile files. A hook must not write directly to arbitrary
paths under `PROFILE_NEW_GENERATION` with plain `mkdir`, `cp`, or shell redirection unless it first proves the target
ancestors are real generation directories. With optimized profile trees, a top-level path such as `core`, `etc`, or
`bin` may be a symlink into `/nix/store`; direct writes through that path can fail on a real Nix store or mutate a
mutable test store.

Prefer built-in hooks for profile-wide generated content. If a new hook must generate files, implement the write in Rust
and use the profile manager's generated-file helper so symlinked ancestors are materialized inside the generation before
the file is written.

## Built-in Hooks

`bmc-hook-merge-files` reads `merge-files/<target>/<fragment>` entries and writes one generated profile file per
`<target>`, concatenating fragments in lexicographic order.

`bmc-hook-file-symlinks` reads JSON definitions from `file-symlinks/` and generates
`core/activation/scripts/60-file-symlinks`, which creates live filesystem symlinks during activation.

`bmc-hook-activation-resolver` reads `core/activation/scripts/` and generates `core/activation/entrypoint`, the script
that runs activation scripts in sorted order.

These hooks use generated-file writes so generated outputs stay in the generation even when their parent paths were
initially symlinks into the store.

## Activation Scripts

Activation runs after a generation has been built and selected for activation. The activation entrypoint receives:

```text
PROFILE_NEW_GENERATION=/path/to/new/N-link
PROFILE_OLD_GENERATION=/path/to/old/M-link
```

`PROFILE_OLD_GENERATION` is empty on first activation. Activation scripts may compare old and new generations, modify
the live root filesystem, start or stop services, and atomically update the profile's `current` symlink at the write
boundary managed by the activation sequence.

Put live-system side effects in activation scripts, not hooks. Put profile-build computation in hooks, not activation
scripts.

## Manifest

The profile manifest is generated after hooks and written to `<generation>/manifest`. It records the packages used to
build the generation, including versions and store paths. Upgrade planning reads manifests from previous generations to
decide what to keep, add, remove, or replace.

The generated manifest is owned by the profile builder. Packages should not provide a meaningful `manifest` file at the
profile root.

## Contributor Checklist

- Add static files to package outputs or `mkPackage` sections.
- Use `hooks/` only for build-time generation that depends on the assembled profile.
- Do not make hooks depend on `current`, `/etc`, running services, or the old generation.
- Do not write through `PROFILE_NEW_GENERATION/<path>` from shell hooks unless symlinked ancestors are handled
  deliberately.
- Use activation scripts for live filesystem and service side effects.
- Treat generation paths as symlink-heavy read-only views except for files explicitly generated by the profile builder
  or built-in hooks.
