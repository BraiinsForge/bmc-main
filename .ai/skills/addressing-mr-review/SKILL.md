---
name: addressing-mr-review
description: Use when working through the review threads on an MR to fix them, one by one — the fix-and-fold loop, not the reply half. Triggers on "we've got a review pass", "address the review", "work through the MR comments", "go through the reviewer's threads one by one", "fix the review feedback", "let's start on the review". For the mechanics of drafting and posting a single reply, defer to `gitlab-mr-reply`; for the `glab` transport, `gitlab-mr-inline-comments`.
---

# Addressing an MR review pass

The loop for turning a pile of reviewer threads into landed fixes. `gitlab-mr-reply` is the *reply* half (draft, post,
resolution rules); this skill is the *fix* half that wraps it — triage, a per-item gate, and the commit discipline that
keeps history clean. Load both.

## Shape

1. **Load + triage.** Pull the MR + all discussions (per `gitlab-mr-reply` §1–3). Turn every real (`system: false`)
   thread into a task with `TaskCreate` — one per thread. Note severity (blocker → nit) and, crucially, **clusters**:
   threads that rewrite the same lines collapse into *one* code change with several replies. Order: blocker/cluster
   first (it often reshapes the code the nits sit on), then quick wins, then product-calls, then test-coverage.
2. **Per-item loop**, below. Do items one at a time; the user gates each.

## The per-item loop

For each task, in order — do **not** batch:

1. **Explainer + your take, grounded in the code as it is now.** Read the file at the thread's `new_path:new_line`, and
   verify the reviewer's claim independently — trace the contract it depends on (e.g. read the backend/proto, not just
   the frontend) before agreeing it's real. Echoing the thread text is not enough. State your take and the trade-offs.
2. **Ask for direction.** Present concrete choices (scope, wording, placement), not a fait accompli — especially for
   product/UX or anything user-facing. Wait for the go.
3. **Implement**, matching surrounding style. For non-trivial behavior, prove it: write a regression spec red→green (see
   `repo-build-workflow`), and/or drive it in the mock and eyeball it (scenario file + `video-frame-extraction`).
4. **Validate before folding** — `just validate` (or the project build skill). Green is a precondition for the fold.
5. **Find the commit target and fold into it *now*.** See "Fold, don't stack" — this is the step that goes wrong.
6. **Reply to the thread** via `gitlab-mr-reply` rules: grounded, terse, **cite the commit by its subject line in
   backticks — never a SHA** (the branch is being folded/rebased, SHAs go stale), attribution footer, and **never
   resolve** the thread (the reviewer resolves). MCP is read-only for notes, so post through `glab`:
   `glab api --method POST "projects/<id>/merge_requests/<iid>/discussions/<thread-id>/notes" -f "body=$(cat body.md)"`
   then confirm the returned `.id`/`.author.username`.
7. **User pushes.** Never push yourself, even after a reply.

## Fold, don't stack

"Find the commit target and fold if fixup" means **squash the fix into the target commit that step**, leaving one clean
commit that grows — not a pile of `fixup!` commits to autosquash later. A branch that accumulates ten `fixup!` commits
is the failure mode.

- Identify the commit that introduced the touched code: `git log --oneline <base>..HEAD -- <path>`.
- Commit the fix, then fold it in. Interactive rebase (`git rebase -i`) is **blocked in this environment**, so:
  - when every pending commit targets the **same** commit, fold losslessly with
    `git reset --soft <target-sha> && git commit --amend --no-edit`;
  - verify nothing changed but the graph: `git rev-parse HEAD^{tree}` must match the pre-fold tree hash.
- Only tracked source is committed; gitignored runtime files (mock scenario JSON, generated `*.scss.d.ts`) stay out.
  Note: adding/renaming a CSS-module class fails `tsc` until the gitignored sibling `*.scss.d.ts` is regenerated (via an
  `rsbuild` run) or hand-synced — the source change is right, the generated type is just stale.

## Rebasing onto the target branch

When asked to rebase onto the MR's target (often after a force-push of that branch): fetch it first, then replay **only
your own commits** with an explicit old base — `git rebase --onto origin/<target> <parent-of-your-first-commit>`. A
plain `git rebase origin/<target>` will try to replay stale copies of the target's own commits when the target was
rewritten (same subject, different SHA). Re-run `just validate` after — the rebase pulls in new base commits that your
stubs/UI must still be coherent with.

## Hard rules — never

- Never leave a stack of `fixup!` commits — fold each into its target at its step.
- Never reference a SHA in a reply; cite by subject line.
- Never resolve a thread unless explicitly told — the reviewer resolves.
- Never `git push` — the user does.
- Never fold before `just validate` is green.
- Never batch replies to the end — reply the moment each item lands.
