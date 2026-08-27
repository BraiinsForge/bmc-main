---
name: finalizing-ticket
description: Use when wrapping up a ticket's work and getting it ready for review — the umbrella checklist that runs before a branch is offered for merge. Triggers on phrases like "finalize the ticket", "wrap this up", "ready for review", "submit for review", "prep the MR", "mark it done", "create an MR", "open an MR", "create a merge request", "open a merge request", or when the implementation is complete and the next step is review. Creating or opening an MR always runs through this skill. For Story-type tickets this skill defers the user-facing documentation step to the `finalizing-story-ticket` skill.
---

# Finalizing a ticket

Run this before offering a branch for review. It is the single checklist that turns "the code works" into "this is ready
for someone else to merge". The steps below apply to every ticket type; the **story addendum** at the end applies only
when the ticket is a Story.

## 1. Definition of Done holds

Walk the Definition of Done from `.ai/instructions.md` and confirm each item is actually true, not assumed:

- tests written and passing for the new behavior
- code follows project conventions, no linter/formatter warnings
- no TODOs without an issue number
- the implementation matches the plan

## 2. Validate the tree

Run the repo's validation path — never raw `cargo`/`nix` substitutes. See the `repo-build-workflow` skill for the
narrower targets worth using mid-iteration. Finish on the full `just validate` and confirm the `validate: OK` marker.

`just validate` does not cover the frontend: on a branch that changes `frontend/`, also run `just fe::validate`
separately and confirm it passes.

## 3. Clean up scratch state

The planning devlog under `docs/devlogs/BDK-<ticket>-<slug>/` is scratch, not deliverable. Remove it once the stages are
done — it must not land in the review. Permanent documentation (architecture notes, user stories) is a separate concern
and lives elsewhere; see the story addendum below.

## 4. Commit hygiene

Each commit compiles and passes tests on its own (verify-at-every-commit). Commit messages follow the repo format —
imperative subject, ticket reference, body explaining *why*. Squash fixups into the commit they belong to so the history
reviewers read is the history that merges.

The commit message is the only place a ticket reference belongs. This is an open-source repo, and `git log` already ties
every line back to its ticket, so strip `BDK-`/`BOS-` mentions from code comments, `docs/`, and protocol XML copyright
blocks before offering the branch. Find the ones this branch added with:

```bash
git diff "$(git merge-base HEAD origin/master)"..HEAD | grep -E '^\+.*(BDK|BOS)-[0-9]'
```

Older references elsewhere in the tree are not this branch's to clean up.

## 5. The story addendum — only for Story-type tickets

Check the ticket type (the Jira `issuetype` is `Story`, not `Task` / `Bug`). If it is a Story, the user-facing feature
documentation under `docs/stories/` is part of finishing the ticket. **Switch to the `finalizing-story-ticket` skill**
for that step before considering the ticket done. Task and Bug tickets skip it unless they change documented user-facing
behavior.

## 6. Push and open the MR are explicit actions

Pushing the branch and opening / updating the MR are deliberate, outward-facing steps. Do them when the work is actually
ready and you intend to publish it — not as a routine "make sure the remote is current" reflex. One authorization covers
one push of the named branch, not a standing licence.

When you do open the MR (with `glab`, run unsandboxed):

- **Assignee is always the person who opened it.** Set the assignee to the authenticated GitLab user (`glab api user`
  gives the current account; pass its `username` to `--assignee`). An MR is never left unassigned.
- **A reviewer is optional.** If the user named a reviewer, set it. If they did not, open the MR without one — never
  block MR creation on a reviewer and never guess one. After the MR exists, ask the user, as a friendly follow-up,
  whether they'd like a reviewer; only add one if they say yes.
- **When a reviewer is set, put it on the MR yourself and nudge the user to mirror it on the Jira ticket.** On the MR
  use `--reviewer <username>` (find the account with `jira api "users?search=<name>"`). The Jira **Reviewers** field
  (custom field `customfield_10118`, an array of user) cannot be written from the CLI — `jira issue edit --custom`
  (jira-cli ≤ 1.7.0) only serializes array-of-`option` fields and returns `400 Invalid request payload` for it. So do
  not try to set it; instead gently remind the user to add the reviewer to the ticket's Reviewers field in the Jira web
  UI.

## Hard rules — never

- Never offer a branch for review with the scratch devlog still committed.
- Never skip `just validate` (the formatter step is load-bearing) before calling a ticket done.
- Never leave a ticket reference in code, docs or a protocol XML copyright block; the commit message is its only home.
- Never finalize a Story ticket without the `docs/stories/` entry — route through `finalizing-story-ticket`.
- Never treat push / MR creation as an automatic finalization step; it is a separate, explicit action.
- Never open an MR with no assignee — the assignee is always the opener.
- Never require a reviewer or block MR creation on one — asking whether the user wants a reviewer is a friendly
  follow-up after the MR exists; a named reviewer goes on the MR, and for Jira just remind the user to set the
  `customfield_10118` Reviewers field (the CLI cannot write it).
