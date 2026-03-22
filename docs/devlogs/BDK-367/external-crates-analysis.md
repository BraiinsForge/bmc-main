# External Crates Analysis: tooling/ subtree

## Current State

The `tooling/` directory is a **vendored git subtree** from
`ssh://git@gitlab.ii.zone/tooling/tooling.git`, pinned at commit `651d341b` in
`crate-verification.config.json`.

It contains 6 crates used primarily by the firmware upgrade system:

| Crate              | Direct consumers                        |
| ------------------ | --------------------------------------- |
| `tooling-std`      | `bmc`, `bmc-mock`, other tooling crates |
| `tooling-std-macros` | `idxgen-data`                         |
| `minerctl-defs`    | `bmc`, `idxgen-data`, `index-bmc`       |
| `idxgen-data`      | `index-bmc`, `index-common`             |
| `index-bmc`        | `bmc`, `bmc-platform`                   |
| `index-common`     | `bmc`                                   |

All are declared as `path = "tooling/..."` in root `[workspace.dependencies]`.
They are **not** workspace members — only pulled in as dependencies.

### CI/Nix integration

- `WORKSPACE_LINTS_IGNORE_PATHS` in `.gitlab-ci.yml` excludes `idxgen-data` and
  `tooling-std` from workspace lint checks
- `flake.nix` excludes all tooling crates from the formatter
- `verify_crates.sh` + `crate-verification.config.json` verify the vendored copy
  matches upstream at the pinned commit

## Proposed Change: Git Dependencies (Option A)

Replace vendored subtree with Cargo git dependencies pointing at upstream.

### Cargo.toml

Replace all 6 path dependencies with git deps at the same revision:

```toml
# Before:
tooling-std        = { path = "tooling/tooling-std" }
tooling-std-macros = { path = "tooling/tooling-std/macros" }
minerctl-defs      = { path = "tooling/minerctl/minerctl-defs" }
idxgen-data        = { path = "tooling/idxgen/idxgen-data" }
index-bmc          = { path = "tooling/crates/index-bmc" }
index-common       = { path = "tooling/crates/index-common" }

# After:
tooling-std        = { git = "ssh://git@gitlab.ii.zone/tooling/tooling.git", rev = "e2868e0b" }
tooling-std-macros = { git = "ssh://git@gitlab.ii.zone/tooling/tooling.git", rev = "e2868e0b" }
minerctl-defs      = { git = "ssh://git@gitlab.ii.zone/tooling/tooling.git", rev = "e2868e0b" }
idxgen-data        = { git = "ssh://git@gitlab.ii.zone/tooling/tooling.git", rev = "e2868e0b" }
index-bmc          = { git = "ssh://git@gitlab.ii.zone/tooling/tooling.git", rev = "e2868e0b" }
index-common       = { git = "ssh://git@gitlab.ii.zone/tooling/tooling.git", rev = "e2868e0b" }
```

Cargo deduplicates — same repo+rev = single checkout.

### Nix Build System

The build uses `pkgs.ii.rust.mkWorkspaceConfig { src = ./.; ... }`. Today,
`tooling/` travels inside `src = ./.;` and Cargo resolves path deps locally.

With git deps, Cargo needs to fetch the repo during build. Nix sandboxed builds
have no network access, so this works **only if** `mkWorkspaceConfig` (via crane,
naersk, or similar) pre-fetches Cargo git deps from `Cargo.lock` before the
sandboxed build.

**Option A** assumes `ii.rust` infra already handles this — most Nix Rust
builders do. If it doesn't, **Option B** adds the tooling repo as a flake input
and patches the source, but this is more complex and likely unnecessary.

### Cleanup

After switching to git deps:

- Delete `tooling/` directory
- Remove `tooling/` entries from `WORKSPACE_LINTS_IGNORE_PATHS` in
  `.gitlab-ci.yml`
- Remove `tooling/` entries from `flake.nix` format exclusions
- Remove `tooling` entry from `crate-verification.config.json`
- Drop `verify_crates.sh` tooling verification (subtree no longer exists)

### Risks

- Main risk: `mkWorkspaceConfig` not handling git deps → discovered at
  `nix build` time
- Mitigation: test with `nix build .#checks.x86_64-linux.build` before full
  cleanup
