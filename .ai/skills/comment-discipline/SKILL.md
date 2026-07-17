---
name: comment-discipline
description: The single source of truth for comment hygiene in this repo — both whether to comment and how. Invoke it whenever you finish an implementation, are about to commit code, or start or continue a code review, and whenever you write or edit a comment. In short: prefer self-documenting code; a comment must earn its place; never restate the code, state the trivial, sprawl, or carry plan-staging / call-graph ("who calls this") notes; cite a specific cause for any workaround; and make comments read as a human wrote them. Triggers on "commit", "code review", "review the diff", "before the fixup", "done implementing", "finished the change", "wrap up", or writing/editing any comment.
---

# Comment discipline

The one place for whether and how to comment. A comment is a liability until it earns its place. **Run a comment pass
whenever you finish a change, before you commit, and through a review.** The default is delete.

## Prefer self-documenting code over a comment

Before writing a comment that explains what a line *does*, make the code say it at runtime. Descriptive code that
executes beats cryptic code plus a comment: it can't drift, and it surfaces on failure instead of sitting silent above
the line.

- **Assertion messages, not inline comments** — the intent prints on failure:

  ```rust
  // don't:
  assert!(reg.is_stale(h, 8));  // age 8s exceeds the 7.5s threshold
  // do:
  assert!(reg.is_stale(h, 8), "age 8 s exceeds the 7.5 s threshold");
  ```

- **`expect("BUG: …")`, not `unwrap()` + a comment** — the reason rides the panic.

- **Descriptive names, `const`s, and test names** (`not_stale_without_prior_success`) that state intent, not a comment
  decoding a bare literal or terse body.

This doesn't abolish comments — a genuinely non-obvious *why* still earns one. It abolishes the comment that exists only
because the code was left cryptic.

## The bar — what survives

A comment survives only if it carries what the code cannot:

- the non-obvious **why** — a choice, a trade-off, a gotcha, the reason this algorithm and not another;
- at most a **short** usage example.

If a competent reader gets it from the code alone, cut it. When unsure, cut.

## Never

- **Restate the code.** `// increment the counter` over `count += 1` is noise. If the names say it, say nothing. Cut
  comments narrating the obvious — a `{}` borrow scope, a widening cast.
- **State the trivial.** A comment nobody would miss is a comment nobody should read.
- **Sprawl.** Terse and to the point — one line by default, a few at most. It is not a book club; a paragraph narrating
  three obvious lines is worse than silence. Don't re-document a fact that already lives where it's defined (an on-disk
  format documented at its definition is not re-documented at every call site) — keep it in one place and point there.
- **Stage plans in code.** Deferred work, "next we'll…", "later X consumes this", TODO-without-an-issue — these live in
  the plan file that is already in the repo, not scattered through the source. Ditto any callout about future changes or
  downstream consumers.
- **Reason about the call graph.** "called by X", "Y uses this", "for the Z path" — who reaches the code is not the
  comment's job. Explain the code's own behaviour and choices.

## Workarounds cite a specific, checkable cause

Never paper over a mistake with vague rationale (`// sometimes the renderer drops a frame`,
`// the tooling can confuse this`). If a workaround is genuinely needed, the comment cites one of:

- a bug URL — upstream issue or internal ticket;
- a version pin — `librsvg < 2.59 drops the alpha channel`;
- a reproducer — steps plus the observed failure mode.

Can't cite one? The workaround is probably wrong. Fix the underlying issue cleanly instead of inventing justification.

## Infra layers name the gate, not the effect

Commenting a feature passthrough in `workspace.nix`, a Cargo `[features]` block, or a build flake: describe **what is
wired**, not what that feature happens to enable at one call site today.

```nix
# don't: turns on the verbose compositor::frame_callback trace
# do:   forwards the trace-frame-callbacks Cargo feature so dev profiles can opt into it;
#       matches the other trace-* forwards in this file
features = ["trace-frame-callbacks"];
```

The infra file lives forever; the instrumentation comes and goes. What a feature *does* belongs in the crate's docs, not
here.

## Human flow — read as a person wrote it, not a machine

Prose form, all judgement calls. The goal: a coder skimming during a busy day finds the relevant part without
reassembling paragraphs. The root cause of every tell below is **column-wrapping** — the machine reflex of filling each
line to ~90 chars and breaking wherever the width runs out. Don't do that. Place each break deliberately at a clause
boundary, and let a clause that fits on one line *stay* on one line (fewer, cleaner lines beat filling the margin).
These machine tells are obnoxious and unwelcome:

- **Skimmability first.** Break at sentence and clause boundaries, never at arbitrary columns. Independent sentences
  each land on their own line; a long sentence wraps at a comma, semicolon, or conjunction — somewhere the reader pauses
  naturally.

- **Balance second.** Within those breaks, keep line lengths roughly even — no 5-char line stacked under a 95-char one.
  Never sacrifice skimmability for balance; this is the second layer.

- **No dangling function word — check the last word of every wrapped line.** A line must not *end* on a word that leads
  the phrase opening the next line: an article (`a`, `the`), a preposition (`to`, `of`, `for`, `as`, `in`, `by`), a
  conjunction (`and`, `or`, `so`), or a determiner/quantifier (`no`, `this`, `its`). Break *before* that word so it
  leads the next line with its noun, and never split a noun phrase (`no widget hook`, `the Deck`) across the break.

  ```text
  don't:  /// … unversioned, since it changes rarely and no
          /// widget hook depends on it.       ← "no" left the line without "widget hook"
  do:     /// … unversioned, since it rarely changes and no hook reads it.
  ```

## The pass

Whenever you finish a change, before a commit, or through a review: reread every comment in the diff and ask — does the
code already say this? is it trivial? is it terse? is it a plan note or a who-calls-this note that belongs elsewhere?
does a workaround cite its cause? does it flow like prose — and does every wrapped line end on a real word, not a
stranded `the`/`a`/`to`/`and`/`no`? Delete or tighten each one that fails. A change that adds code should usually delete
more comment than it adds.
