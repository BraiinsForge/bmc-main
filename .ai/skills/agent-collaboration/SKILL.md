---
name: agent-collaboration
description: Use whenever interpreting a request that might be ambiguous about whether to commit, presenting options that differ in cost, or porting/refactoring existing code into a new shape. Triggers on phrases like "track this", "record this", "persist this", "commit", "let's go with option", "what are the options", "rewrite to", "refactor for borrow checker", or when working through a list of alternative approaches.
---

# Agent collaboration rules

Three norms about how an agent works *with* a human on this codebase. These are repo-tracked because they apply to
anyone driving an agent on this project — not just to the original author's personal preferences.

## Never `git commit` without an explicit, unambiguous ask

Only run `git commit` when the user has explicitly asked for a commit in those terms. The following phrasings do **not**
authorize a commit; they describe artifact durability or organizational intent:

- "committed memory" / "committed record" — about persistence, not the git verb.
- "track this" / "record this" — describes the act of capturing the information.
- "persist this" / "save this for later" — same.
- "save to memory" / "remember this" — about the agent's memory system, not git.
- "make a note of" / "keep this" — note-taking, not committing.

When the request is about creating / organizing / archiving files: write them, `git add` them if it helps, then **stop
and ask** before committing. If the user wants a commit they will say "commit", invoke `/commit`, or be unambiguously
explicit. No clever interpretation; no "well, they probably meant…".

This is a one-way ratchet: commits, amends, and history-rewrites stay user-driven. If you notice unexpected git state
(missing commits, unstaged work, divergence), report it plainly and stop. Don't propose a fix, don't describe
alternatives, don't ask a yes/no that pushes toward action. Stay out of the user's git workflow until explicitly
invited.

## Don't pre-eliminate options as "out of scope"

When sizing options for a fix or a refactor, never silently filter one out with framings like "out of scope for this
MR", "too big to do here", or "separate ticket". Scope is the user's call, not yours.

The right shape:

- Present every realistic option, including the inconvenient ones.
- Quote **honest cost** for each: files touched, blast radius, downstream consumers, ABI/FFI surfaces, lock-file
  refreshes, etc.
- If an option is genuinely large, say *why it's large* in concrete terms (number of impls, downstream consumers,
  cross-workspace effects) — not "this is too big".
- Make a recommendation as a recommendation, not a filter. End-state: the user can pick the inconvenient option if the
  trade-off is worth it to them.

This applies to MR review responses, refactor proposals, plan documents, and any "what should we do about X?" round.

## Don't speculatively rewrite to dodge imagined errors

When porting or refactoring code that already compiles for its original author, port it verbatim first and let the
compiler tell you what actually breaks. Don't preemptively restructure to "fix" borrow-checker patterns, lifetime
issues, or other errors you think might come up.

Rust's NLL (non-lexical lifetimes) splits shared borrows at last use. Patterns like:

```rust
let x = state.foo.get(&id);
state.bar.push(format!("...{}...", x.cloned().unwrap_or_default()));
```

are usually fine — the argument is evaluated (and the shared borrow of `x` ends) before `push` takes `&mut state.bar`.
Preemptively rewriting to "snapshot fields up front then mut-access" produces cosmetic muck-up and often forces
unconditional clones on branches that never needed them.

The general form applies beyond borrow-checking: if a change is cosmetic in already-working code and you can't name a
concrete runtime, correctness, or perf benefit, don't make it. Cosmetic-looking rewrites burn review time and introduce
regressions that didn't exist in the original.

**How to apply:**

1. Port verbatim.
2. Run `just validate`.
3. Read the *actual* compiler error if any.
4. Only then restructure — and only as much as the error demands.

## Hard rules — never

- Never `git commit` / `git commit --amend` / `git reset` / `git restore` / `git rebase` / `git stash` / `git branch -D`
  unless the user explicitly asked. Words like "track", "record", "persist", "save", "remember" do not count.
- Never silently filter out an option as "out of scope". Size honestly; the user decides.
- Never preemptively restructure already-working code to dodge an error you haven't yet seen the compiler produce.
- Never propose destructive operations as a "fix" for unexpected git state without an explicit user ask — state the
  observation and stop.
