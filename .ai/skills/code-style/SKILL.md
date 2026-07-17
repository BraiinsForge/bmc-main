---
name: code-style
description: Use when writing or modifying code in this repository — picking quantitative defaults (fps, fuel budgets, timeouts) without inventing them, choosing idioms and libraries over ad-hoc code, Rust `use`-block consistency, and small cross-language form rules (number separators, redundant block-condition parens, `node:` prefixes). For comment hygiene — whether to comment and how — use the `comment-discipline` skill instead. Triggers when picking a default/knob value, adding a `use` block, reaching for an ad-hoc implementation, or writing a numeric literal.
---

# Code style in this repository

Form and choice rules that bite during review but aren't loud enough for the formatter or linter to catch.

> **Comments live elsewhere.** Everything about comments — whether to write one, terseness, self-documenting code over
> comments, workarounds citing a cause, infra-layer comments, and prose form — is the `comment-discipline` skill. This
> skill covers the non-comment rules below.

## Quantitative defaults — cite or expose

When choosing a frame cadence, fps target, fuel budget, memory ceiling, timeout, or any other numeric knob, **either
cite a project source or expose the value as configuration**. Don't default to a "common value" (e.g. `16ms = 60fps`)
just because it feels familiar.

Before picking a number:

1. Search `docs/` and `docs/devlogs/` for the relevant target.
2. If multiple sources conflict, surface the conflict to the user and let them pick. Don't silently average or pick one.
3. For runtime code, prefer exposing the knob via an existing config surface (`RuntimeConfig`, etc.) so different host
   integrations can override.

There is **no single project-wide cadence**. The docs encode different targets at different layers — `BDK-266` NFR sets
60fps, `BDK-141` compositor-cpu analysis observed ~40fps on GC400, `BDK-355` mesh budget targets 30fps for 3D widgets —
so picking one number unilaterally bakes in an unfounded assumption. Look up the current state of the docs before
relying on any specific value.

## Rust — `use` block consistency

Don't mix `self` with named selective imports in the same `use` group:

```rust
// don't:
use foo::{self, A, B, C};

// do — either bring the function in by name:
use foo::{A, B, C, func};

// or take the module path on a separate line:
use foo;
use foo::{A, B, C};
```

One style per `use` block keeps grep/rename predictable and avoids the reader asking "why is `foo::` qualified here but
`B` is bare?".

## Rust — prefer idioms and libraries over ad-hoc code

Choices that shape the code beyond its form, collected from review feedback:

- **Don't reinvent the wheel.** Use a library when one exists for the task. More code needs justification — prefer a
  well-tested, well-known third-party crate to an ad-hoc implementation.
- **Unsafe needs a reason.** Every `unsafe` block needs a justification a reviewer can check — same bar as the
  workaround-comment rule in `comment-discipline`.
- **Use the standard conversion traits.** Reach for `From` / `Into` rather than ad-hoc `from_*` / `to_*` inherent
  methods, so conversions compose and read the same way across types. Prefer traits generally when one fits.
- **`Option`, not sentinels.** Model absence with `Option<T>`; never encode "none" as `0`, `-1`, or an empty string. The
  type should make the absent case impossible to read past by accident.

## Cross-language form rules

Small rules that have come up in review:

- **Number separators.** Use the language's separator (`1_000_000` in Rust and modern JS, `1_000_000` in Python ≥ 3.6,
  etc.) on numeric literals of four digits or more. The formatter doesn't enforce this; humans skim large numbers wrong
  without separators.
- **No redundant block-condition parens.** Where the language permits omitting them around `if`/`while`/`for`/`match`
  conditions, omit. `if x { ... }`, not `if (x) { ... }` — Rust, Swift, Go, and Kotlin all allow the bare form;
  clippy/IDE warnings will flag the parens for you when the formatter misses them.
- **`node:` prefix for Node builtins.** Always `import { readFile } from "node:fs/promises"` — never
  `from "fs/promises"`. The prefix disambiguates against npm-published shadows and signals intent to the reader.
