---
name: code-style
description: Use when writing or modifying code in this repository — covers comment prose form (skimmability and balance), what infra-layer comments may and may not say, when a workaround comment is acceptable, picking quantitative defaults (fps, fuel budgets, timeouts) without inventing them, Rust `use`-block consistency, and small cross-language form rules. Triggers when writing a comment, picking a default value, adding a `use` statement, or commenting a feature passthrough in `workspace.nix` / Cargo.
---

# Code style in this repository

Form rules that bite during review but aren't loud enough to be caught by the formatter or linter. The repo-root
`CLAUDE.md` governs *whether* to write a comment; this skill governs *what it should look like* when you do, plus a
handful of cross-language form rules collected from review feedback.

## Comments — prose form

Two layered concerns, both judgement calls. The goal: *a coder skimming during a busy day can find the relevant part
without reassembling paragraphs*.

### Primary — skimmability

Line breaks should help the scanner. Break at **sentence and clause boundaries**, not at arbitrary columns:

- Independent sentences each land on their own line.
- A long sentence wraps at a comma, semicolon, or conjunction — somewhere the reader can pause naturally.
- One mid-thought arbitrary wrap forces the reader to reassemble the paragraph. Don't.

### Secondary — balance

Within the semantic breaks above, prefer roughly even line lengths. Avoid 5-char-then-95-char asymmetry — think CSS
`text-wrap: balance`. Never sacrifice skimmability for balance; this is the second layer, not the first.

### Length — one line by default

Most comments are a single line. Reach for a second only when a distinct, non-obvious fact needs it — never to restate
the code, the type name, or a doc that already carries the fact (an on-disk format documented where it is defined does
not get re-documented at every call site). Cut comments that narrate the obvious: a `{}` borrow scope, a widening cast.
When the same fact would live in two places, keep it in one and point to it.

## Comments — workarounds must cite the cause

Never paper over a mistake with vague rationale:

```rust
// sometimes the renderer drops a frame here
// in some cases the tooling can confuse this
```

If a workaround is genuinely needed, the comment must cite a **specific, checkable cause**:

- A bug URL (upstream issue, internal ticket).
- A version pin (`librsvg < 2.59 drops the alpha channel`).
- A reproducer — steps + observed failure mode.

If you can't cite one, the workaround is probably wrong. Fix the underlying issue cleanly instead of inventing
justification.

## Comments — infra layers name the gate, not the effect

When commenting a feature passthrough in `workspace.nix`, a Cargo `[features]` block, or a build flake, describe **what
is being wired**, not **what the wired feature happens to enable at one particular call site right now**.

Avoid:

```nix
# turns on the verbose compositor::frame_callback trace
features = [ "trace-frame-callbacks" ];
```

Prefer:

```nix
# forwards the trace-frame-callbacks Cargo feature so dev profiles can opt into the
# instrumentation; pattern matches the other trace-* feature forwards in this file
features = [ "trace-frame-callbacks" ];
```

The infra file lives forever; the instrumentation comes and goes. If a reader needs to know what a feature actually
does, they read the crate's docs — not `workspace.nix`.

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
  workaround-comment rule above.
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
