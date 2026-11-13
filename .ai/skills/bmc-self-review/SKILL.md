---
name: bmc-self-review
description: Use when reviewing your own committed branch in this repository before offering it for review or opening an MR — when the `bmc-finalizing-ticket` skill offers it, or when asked to. Triggers on "self-review the branch", "review my branch", "review the branch", "is this ready", "check my changes".
---

# Self-review

Find what a teammate would flag on the MR, before anyone else reads the diff. The checklists in this directory are the
findings that keep coming back in this repository's reviews. Review your own branch to the standard you would use on
someone else's: being the author never lowers it.

## 1. Scope

The review judges committed work. When `git status --porcelain --untracked-files=no` lists anything, stop and ask for it
to be committed first: a finding on uncommitted code can't be placed as a fixup. Judge the history as it will merge: a
`fixup!` is part of the commit it names, and an `amend!` message replaces that commit's message.

```bash
target=master   # the MR's target branch: a release branch for a backport
git fetch origin "$target"
base=$(git merge-base HEAD "origin/$target")
git log --oneline "$base"..HEAD
git diff --stat "$base"..HEAD
git diff --name-only "$base"..HEAD
```

If the fetch fails, use the existing `origin/$target` and say so in the report.

The review covers everything `git diff "$base"..HEAD` shows. Code that was already there at `$base` is out of scope
unless the diff calls it, copies it or changes what it means.

## 2. Route the checklists

Read the checklists that match the changed paths:

- any `*.rs` → [rust.md](rust.md)
- any code or test change → [tests.md](tests.md)
- any code or asset change, and `*.nix`, `flake.lock`, `Cargo.toml`, `Cargo.lock`, `justfile`, `*.justfile`, `*.just`,
  `.gitlab-ci.yml` → [build.md](build.md)
- `frontend/` → [frontend.md](frontend.md)
- always → [architecture.md](architecture.md) and [docs-commits.md](docs-commits.md)

## 3. Sweep with a fresh agent

You read the diff for what you meant; a fresh reader sees only what the code does. Dispatch one subagent with no
conversation history, on the session's model unless the author names another. Its brief contains:

- the base SHA and the branch;
- the paths of the routed checklists, to read in full;
- the ticket's acceptance criteria, if there is a ticket;
- the required output, one block per changed file, and in a code file one block per changed function or type. A block
  for a file with no code in it (docs, Nix, CI, a manifest) holds only the findings slot:
  - **closest existing code**: the `path:line` of the closest existing implementation outside the diff, found by
    searching for its key operations (the arithmetic, the constants, the calls it makes), not just its name. Otherwise
    write "none", followed by the searches that were run;
  - **findings**: `path:line` — the defect — a concrete input or state that triggers it — the checklist item;
  - **bug hunt**: what happens on the first call, the second call, the error path and the boundary values;
- for each new or changed test: the one-line change to the production code it should catch, and whether its assertion
  depends on that change.

If no subagent is available, do the sweep yourself, one file at a time, filling in the same slots.

## 4. Verify every finding

A finding stays a claim until you check it:

- open `path:line` and confirm the code says what the finding says;
- for a duplication, open the named implementation and compare the two;
- for a test the sweep says can't fail, read the test and the code it covers, and confirm that its assertion would hold
  under the named change;
- a behavioural finding you can't trigger with a concrete input or state is dropped. A structural finding (a type that
  allows a bad state, a `pub` item with no caller, a long file with a seam to split along, a commit without a why)
  stands on the code it points at.
- a file or symbol a checklist names that no longer exists is a stale pointer: search for the pattern instead, and name
  the pointer in the report so the checklist gets fixed.

## 5. Classify

- **Critical**: data loss, a security hole, a crash or a bricked device on a plausible path, shipped persisted state
  that no longer loads.
- **Major**: wrong behaviour on a plausible path; a new production panic the types don't guarantee; a panic that depends
  on the caller; a copy of existing logic that can drift; a test that can't fail; a claim the branch adds or changes in
  a doc, comment or commit that the code doesn't back; an unmet acceptance criterion; an incompatible change to a
  versioned format (see [architecture.md](architecture.md)) without a version bump.
- **Minor**: type design (state spread across fields, a mode `bool`, a sentinel, the wrong integer width); dead or
  speculative API; logic in the wrong layer; an unnamed constant; code that is hard to read, such as nested languages in
  one literal or JSON built with `format!`; a doc the branch left outdated; a commit without a why, or mixing concerns.
- **Nit**: naming, wording, comment form.

These escalations always apply:

- Type design is Minor or above, never a Nit.
- A new `expect`, `unwrap` or `unreachable!` in production code, or an index of the kind [rust.md](rust.md) names, needs
  a justification: why the invariant behind it can't be enforced at compile time. Without one it is Major, and the fix
  is to make the type carry the invariant. The common case is checking an `Option` and then calling `expect` on it
  again, instead of keeping the value that `if let`, `let ... else` or `match` already gave you. With a sound
  justification it is not a finding, and the `expect("BUG: ...")` message states it.
- Tag each finding **INTRODUCED** (the branch adds the defect, or makes an existing one reachable) or **PRE-EXISTING**
  (it was already there at `$base`, and the branch doesn't change who reaches it). A pre-existing finding ranks one
  level lower.

## 6. Not findings

Drop these and list them under "Not worth chasing", one line each with the reason:

- a failure that needs a broken system: `uci`, sysfs, or a shipped binary, missing or failing;
- input that can't occur, or that only an attacker who already has root on the device can produce;
- a lint or format point a gate reports on its own: rustfmt and clippy through `just validate`, Biome through
  `just fe::validate`, which a branch that changes `frontend/` runs as well, and cargo-deny through its CI job. Drop it
  only when that gate actually runs on the changed target. A gate that fails because of this branch is a finding;
- a migration for a persisted or wire format that no release ever wrote. Releases are the `stable-*` branches, not tags,
  and they take backports as cherry-picks, so search them for the format, not for the commit: after `git fetch origin`,
  `git grep -n '<field or type>' $(git branch -r --list 'origin/bmc/stable-*' 'origin/bmcpkgs/stable-*')` finds nothing;
- an integer width set by an external API, a wire format or hardware;
- behaviour the ticket explicitly specifies;
- a render that is wrong only on a viewport the change does not target. The Deck viewports set the bar, and BMM100 is
  not one of them;
- coverage for its own sake: a missing test is a finding only when it leaves behaviour or a claim unproven.

## 7. Report

```text
Self-review of <branch> against <base>: 1 Major, 1 Minor, 1 Minor pre-existing. Not ready for review.

1. Major INTRODUCED bmc/src/foo.rs:120 — <finding> — <the input or state that triggers it>.
2. Minor INTRODUCED bmc/src/foo.rs:40 — <finding>.
3. Minor PRE-EXISTING bmc/src/bar.rs:300 — <finding>.

Not worth chasing:
- bmc/src/foo.rs:88 — <claim>: <reason it was dropped>.
```

The tally line counts only verified findings. Order them by severity, highest first. The verdict is `Ready for review.`
when no Critical or Major finding is left, and `Not ready for review.` otherwise. A finding that depends on a decision
(the ticket's scope, a policy, a choice between designs) says so.

## Applying the findings

The review changes no code. Work through the findings with the `bmc-review-addressing` skill, which takes them one at a
time and places each fix as a new commit or as a `--fixup` of the commit it corrects. Then run the review again.

## Rationalizations

| Thought                                           | Reality                                                                                    |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| "Nothing in the repo does this already"           | Not until you name the searches. Grep the arithmetic and the constants, not just the name. |
| "The test obviously covers it"                    | Name the change it should catch. Tests asserting a value against itself pass regardless.   |
| "The server always sends a valid value"           | Input from outside the process is plausible input. An `expect` on it is Major.             |
| "The `debug_assert` guards it"                    | Release builds compile it out. On the device, it guards nothing.                           |
| "Fixing it would touch five commits"              | Fixups fold the churn. Effort is not a reason to drop a finding.                           |
| "It's only a Nit" about a `bool` that holds state | Type design is Minor or above.                                                             |
| "It's my own branch, I know what it means"        | A reviewer doesn't. Review it the way they will.                                           |
