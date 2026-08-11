---
name: repo-build-workflow
description: Use when validating code locally, picking which `just` target to run, deciding whether to reach for raw cargo/nix commands, or making changes that touch the `bmc-wasm-runtime/` SDK or examples. Triggers on phrases like "validate the changes", "run clippy", "check if it builds", "before the fixup", "is this lint-clean", or when about to touch `bmc-wasm-runtime/sdk/Cargo.toml`, `widgets-wasm-examples/`, or workspace structure.
---

# Repo build & validation workflow

The repo wraps every routine validation step in `just` recipes. Use them. Reaching for raw `cargo check`,
`cargo clippy`, `cargo test`, or `nix fmt` directly bypasses load-bearing pre/post-steps and ends with surprises in CI
or on the next push.

## The default — `just validate`

After **any** edit — Rust, Markdown, justfile, TOML, Nix, docs, configs — run plain `just validate` before creating a
fixup or proposing the change is "done".

`just validate` runs, in order:

1. `nix fmt` — workspace formatter across Rust, Nix, Python, Shell, Protobuf, TOML, YAML, Markdown.
2. Cargo clippy across the full workspace, all features, all targets, with `-D warnings`.
3. Repo content checks (the `content` nix check).
4. Final marker: `@echo "validate: OK"` — if you don't see that line, validate failed or is still running.

The frontend is not part of `just validate`: on a branch that changes `frontend/`, run `just fe::validate` separately.

Why it's not "just clippy": the formatter step is load-bearing. `cargo check` / `cargo clippy` / `just clippy` alone
leave the tree in a "compiles but will fail CI format check" state. Skipping the format step is the most common reason a
fixup gets bounced.

### Invoke it as a bare command

`just validate` is on the agent permission allowlist as a **bare command**. Don't wrap it:

- ✗ `just validate && echo done`
- ✗ `just validate; echo EXIT=$?`
- ✗ `just validate 2>&1 | tee out.log`
- ✓ `just validate`

Shell chaining or redirection turns it into a different command string, breaks the allowlist, and surfaces a permission
prompt every time. If you want a status line, modify the recipe instead of wrapping the invocation. The `validate: OK`
line at the end is the success marker; nothing else needed.

## Narrower targets

These are **supplements**, not substitutes for `just validate`. Always finish with the full one.

- `just clippy` — clippy only, no formatter, no content checks. Use during tight iteration; finish with `just validate`.
- `just format` / `nix fmt` — formatter only. Use when you want to format-only on a partial change before continuing to
  iterate.

The pattern: focused target during the loop → `just validate` before the fixup.

## Just targets over raw tool commands

The justfile is the single source of truth for how validation/format/lint tools are invoked in this repo. The user
maintains it, may change flags, ordering, or wrappers over time, and the justfile changes propagate to everyone. Running
raw `cargo …` / `nix fmt` / `biome …` directly bypasses that.

Before running any dev tool, check whether a justfile target exists for it (`just --list` at the repo root, or the
nearest module justfile). Apply this across all justfiles — root, `bmc-virt/`, `harness/`, etc.

## Wasm SDK ↔ examples lock coupling

The repo has **three** separate cargo workspaces with their own `Cargo.lock` files:

1. Root workspace.
2. `bmc-wasm-runtime/sdk/`.
3. `widgets-wasm-examples/`.

The `wasm-regression` CI job builds the examples workspace with `--locked`. When the SDK gains a new path dep (e.g.
`bmc-led`), the new package needs to be in `widgets-wasm-examples/Cargo.lock` because the examples consume the SDK
transitively. CI's `--locked` flag refuses to auto-update — the job fails with
`cannot update the lock file ... because --locked was passed`.

**`just validate` does not exercise the examples workspace's `--locked` path.** Any change to
`bmc-wasm-runtime/sdk/Cargo.toml` deps (or to root `Cargo.toml` if the SDK depends on a workspace member) requires
refreshing the examples lock locally:

```
(cd widgets-wasm-examples && cargo check --target wasm32-unknown-unknown -p hello-widget)
```

Squash the lock-file change into the same commit that introduced the dep so verify-at-every-commit holds.

## Don't add nested cargo workspaces

Hard no-go. The project is actively paying down the *existing* nested workspaces (`widgets-wasm-examples/`,
`sdk-macros/`, `sdk/`, `skin/`, `skin/tools/`) — the desired end-state is a single root cargo workspace. Adding another
nested `[workspace]` runs counter to that direction.

If nix's `mkWorkspaceConfig` / `workspacePath` machinery doesn't fit because a target crate is a path-dep within the
root workspace (no own `Cargo.lock`), the answer is **not** to give that crate its own `[workspace]`. Use
`pkgs.runCommand` + `pkgs.rustPlatform.importCargoLock` against the root `Cargo.lock`, or another mechanism that keeps
the source tree single-workspace.

## Dirty tree triggers full nix rebuilds

Nix flakes hash the git tree to identify source. A dirty working tree changes that hash on every edit, which invalidates
cached binary builds. Anything that calls `nix build "$WORKSPACE#bmc-openwrt-$PROFILE"` (notably the `bmc-virt/` flake)
will rebuild `bmc-openwrt` from scratch on every iteration if the tree is dirty.

When iterating on `bmc-virt/` configuration: commit `bmc-openwrt` binary changes first, so the cached binary build is
reused while you iterate on `bmc-virt/flake.nix` separately. Otherwise you're paying for a full ARM cross-compile per
hot-reload of an unrelated config file.

## Hard rules — never

- Never substitute `cargo check` / `cargo clippy` / `cargo test` / `nix fmt` for `just validate`. The formatter step is
  load-bearing.
- Never wrap `just validate` in shell chaining or redirection — breaks the agent permission allowlist.
- Never propose a new nested cargo workspace as a workaround.
- Never use `--no-verify` to bypass commit hooks. Fix the hook failure root cause instead.
- Never skip refreshing `widgets-wasm-examples/Cargo.lock` after an SDK dep change; CI will catch it with `--locked`,
  but the human reviewer will catch it first.
