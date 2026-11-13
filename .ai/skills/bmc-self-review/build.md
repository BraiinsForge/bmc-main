# Build checklist

- **Locks move with their manifests, in the same commit.** `Cargo.lock` is current after every commit: a `Cargo.toml`
  change that moves dependency resolution comes with its lock change. A dependency change comes with every fixed-output
  hash it affects, on every platform that carries one. Each commit builds on its own, so a lock fixed two commits later
  is still a broken commit.
- **Build what CI doesn't.** A flake output or build profile the diff affects that no job in `.gitlab-ci.yml` builds is
  built locally. After a rebase, rebuild in the CI environment (`nix develop .#ci`) when the toolchain or inputs moved.
- **Filesets carry every input.** A new file a build or check reads (an embedded asset, a manifest, a tool config) is in
  that derivation's fileset. Read the fileset, don't infer it from a passing local build.
- **CI runs it.** A new check, test target, or package output is built by a blocking job in `.gitlab-ci.yml`. A package
  output isn't a check, and an `allow_failure` job doesn't gate. A target CI never runs rots.
- **New dependencies.** A new crate needs a reason the standard library and existing workspace dependencies don't cover.
  The `unused-deps` CI job is disabled, so run `nix shell .#pkgs.cargo-machete -c just cargo-machete` inside
  `nix develop` and judge only the hits in crates whose `Cargo.toml` the branch touched. The rest predates the branch.
- **Nix style.** A new `.nix` file takes its dependencies as `callPackage` arguments (`{ lib, stdenv, buf, ... }:`), not
  `{ pkgs }:` through `import`, even where older files use `import`. An override that relies on an upstream version or
  attribute asserts it (`lib.assertMsg (lib.versionAtLeast …)`).
- **New builder logic goes in `nix/`, not `workspace.nix`.** A new script, manifest parsing or install step goes in its
  own file under `nix/`, taken through `callPackage`, and `workspace.nix` keeps only the call.
- **Pins to unmerged work move before merge.** While the branch is in review, a flake input or git dependency may point
  at another repository's open MR branch. Before it merges to master, the pin moves to a merged branch or a tag. The
  self-review doesn't raise a feature-branch pin as a finding. The MR description names it, so the pin gets updated
  before merge.
- **LFS assets go through a checked include, never bare `include_bytes!`.** A machine without git-lfs holds a short text
  pointer in the asset's place, and `include_bytes!` embeds it silently. `include_bitmap!`, `include_audio!`,
  `include_mesh!`, `include_nine_patch!` and `include_skin!` refuse a pointer. `include_svg!` doesn't need to, since no
  SVG is LFS-tracked. Other files use `include_png!` or `include_lfs!` from `bmc_shared_utils`.
- **Just recipes** follow the `justfile-conventions` skill.
