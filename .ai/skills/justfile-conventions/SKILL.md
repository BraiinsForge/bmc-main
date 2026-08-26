---
name: justfile-conventions
description: Use when authoring or modifying a `justfile` in this repository — adding a recipe, adding a new justfile, working inside a `mod`-imported submodule justfile, or wondering why a path resolves to the wrong directory. Triggers on phrases like "add a just recipe", "new just target", "edit the justfile", "fix the just module", or when about to write `source_directory()` / `justfile_directory()` inside a submodule justfile.
---

# Justfile authoring conventions

Three rules that have each bitten the repo. Use of `just` targets (which one to run, when) lives in
`repo-build-workflow`; this skill is about *authoring* the recipes.

## Single-line `#` comments above recipes only

Never write a multi-line `#` comment block immediately above a `just` recipe. The formatter (run as part of `nix fmt` /
`just validate`) reformats multi-line blocks by inserting a blank line into them, which splits the comment into two and
strands lines away from the recipe they were describing.

```just
# bad — the formatter will insert a blank line and split this:
# build the artifact and copy it into the running VM's
# overlay so procd respawns the service with the new bits
deploy:
    ...

# good — single line, no formatter mangling:
# build the artifact and push it into the running VM
deploy:
    ...
```

If the explanation genuinely needs more space, put it in `.ai/instructions.md`, a docs page, or a script the recipe
shells out to — never as a comment block above the recipe itself.

## `mod`-imported justfiles: `source_directory()`, not `justfile()`

In a `just` submodule justfile loaded via `mod foo 'foo/justfile'`, both `justfile()` and `justfile_directory()` resolve
to the **entry-point root** justfile — *not* the imported file. This is deliberate per the `just` docs, and it's a
footgun if you assume "current file" semantics.

For paths anchored to the file a recipe is *defined in*, use `source_directory()` (or `source_file()`):

| Inside `bmc-wasm-runtime/justfile` imported as `mod wasm` from root | Resolves to                       |
| ------------------------------------------------------------------- | --------------------------------- |
| `justfile_directory()`                                              | `<repo-root>` — wrong             |
| `parent_directory(justfile())`                                      | `<repo-root>` — wrong, same cause |
| `source_directory()`                                                | `<repo-root>/bmc-wasm-runtime`    |

**Pattern:** at the top of any submodule justfile, anchor with `source_directory()` for module-relative paths:

```just
# bmc-wasm-runtime/justfile
mod_dir := source_directory()

dev widget:
    cd {{ mod_dir }} && cargo run -p {{ widget }}
```

When debugging a "wrong path" / "no such file" error in a module recipe, the first hypothesis is `justfile()` vs
`source_*()` confusion — not a flag or typo issue.

## Every justfile imports `common.justfile`

`just` runs the first recipe in a file when invoked with no arguments. For a submodule that means `just <module>` runs
the module's *first* recipe — which is often expensive or destructive (`clean`, `update-cache`, `validate`,
`build-everything`). `set default-list := true` in `common.justfile` closes that: bare `just`, `just <module>`, and a
direct `just --justfile <path>` all print the recipe list instead. No `default` recipe anywhere.

That file also carries the settings every justfile shares — `CARGO_TARGET_DIR`, `RUFF_CACHE_DIR`, `RUST_LOG`,
`FORCE_COLOR`, `NIX_SYSTEM` — so a recipe behaves the same however it was reached.

```just
import '../common.justfile'
```

Settings cross an `import` but **not** a `mod`, so the import goes in every justfile individually — including one that
is itself a `mod` of another (`bmc-virt/harness/justfile`). Miss it and the file silently gets the footgun back.

`default-list` needs just >= 1.52.0. The dev shell pins one through the `nixpkgs-just` flake input, but a `just` on
`PATH` outside it is the developer's own, and an older one fails to parse every justfile here. `set minimum-version`
would say so plainly, except it only arrived in 1.55.0 — too late to help the versions that need the warning.

The one exception is `bmc-virt/rootfs/overlay/root/justfile`, which ships to the VM as `/root/justfile` where
`common.justfile` does not exist. Keep its first recipe a harmless one by hand.

Anchor the shared paths with `source_directory()`, never `justfile_directory()`: inside an imported file the former is
stably `common.justfile`'s own directory, while the latter follows whichever justfile was invoked and forks the layout
between entry points. The same asymmetry means a *recipe* body cannot be shared this way — `source_file()` inside an
imported recipe names `common.justfile`, not the importer.

## Hard rules — never

- Never put a multi-line `#` comment block above a `just` recipe — single line only.
- Never use `justfile()` or `justfile_directory()` inside a `mod`-imported submodule justfile when you want the
  submodule's own path. Use `source_directory()`.
- Never add a justfile without `import`ing `common.justfile`. Without it `set default-list` does not apply and a bare
  `just <module>` runs the first recipe.
