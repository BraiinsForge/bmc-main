---
name: justfile-conventions
description: Use when authoring or modifying a `justfile` in this repository — adding a recipe, adding a private default, working inside a `mod`-imported submodule justfile, or wondering why a path resolves to the wrong directory. Triggers on phrases like "add a just recipe", "new just target", "edit the justfile", "fix the just module", or when about to write `source_directory()` / `justfile_directory()` inside a submodule justfile.
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

## Private `default:` recipe at the top

`just` runs the first recipe in a file when invoked with no arguments. For a submodule that means `just <module>` runs
the module's *first* recipe — which is often expensive or destructive (`clean`, `update-cache`, `validate`,
`build-everything`). To prevent the footgun, every justfile — root and submodule — gets a private `default` recipe at
the top that just lists what's available:

```just
[private]
default:
    @just --justfile {{ justfile() }} --list
```

Use `[private]` so it doesn't pollute the `just --list` output that the recipe itself produces. Always reference the
justfile explicitly via `--justfile {{ justfile() }}` — a bare `just --list` invoked from inside a submodule lists
*root* recipes, not the module's.

When adding a submodule justfile (via `mod foo 'foo/justfile'`), `default:` is the first thing in the file. The
expensive recipe goes after it.

## Hard rules — never

- Never put a multi-line `#` comment block above a `just` recipe — single line only.
- Never use `justfile()` or `justfile_directory()` inside a `mod`-imported submodule justfile when you want the
  submodule's own path. Use `source_directory()`.
- Never let the first recipe in any justfile be one that does real work. The first recipe is a private `default:` that
  runs `just --list`.
