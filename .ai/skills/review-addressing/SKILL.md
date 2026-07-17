---
name: review-addressing
description: Work through inbound code-review feedback and CI failures on an MR/PR branch, one item at a time with the user in the loop — the counterpart to authoring a review (that's `code-review`). Use when the user says to address, go through, or handle the review threads, reviewer comments, or CI on their branch (e.g. "load the review and CI and go one by one", "let's address the review", "work through the MR comments"). Covers load threads+CI, parse the addressable set, a terse digest, a gitignored tracker + tasklist, then per item — explain the feedback, verify it against the code, recommend, and ask where to take it; on settling, implement, run comment-discipline, self-review, place the change (new commit vs a --fixup with target-finding and a trial fold), and offer to draft+post the thread reply only on confirm. Never commit/rebase/push/post/resolve without an explicit go for that specific action.
---

# Addressing review feedback

The inbound counterpart to `code-review`: you have a branch under review and need to work through the reviewer's threads
(and the branch's CI). The whole point is **one clear decision at a time with the user**, not a report — so keep the
prose terse and never run ahead.

## The shape

Load → parse the addressable set → terse digest → gitignored tracker + tasklist → then walk the list **one item at a
time**, each item fully closed before the next: agreed with the user → implemented → comment-passed → placed → reply
offered/posted.

## 1. Load

Pull the review threads and the branch's CI status together:

- GitLab: `gitlab_get_merge_request` (grab `diff_refs` — base/start/head sha — you'll need them for inline replies
  later), `gitlab_mr_discussions`, `gitlab_list_merge_request_pipelines` → failed jobs via `gitlab_list_pipeline_jobs` →
  `gitlab_get_pipeline_job_output`.
- GitHub: the `gh` CLI equivalents.

## 2. Parse the addressable set

- Drop the system noise (assigned, "added N commits", label/milestone changes). Keep human review notes and real CI
  failures.
- Note authorship, but don't let it lower your guard: some review notes were drafted by an agent and only vetted by the
  reviewer — still verify each one in the code (a drafted finding can be wrong).
- Bucket by severity and file so the digest is scannable.

## 3. Terse digest

A very short summary to the user: counts by severity, one line per item, CI failures named. This is a map, not the
territory — resist restating each thread in full.

## 4. Tracker + tasklist

- Write a gitignored tracker at `.claude/<TICKET>-review.local.md` (per-user, never committed): one stable ID per item,
  the reviewer's point, your take, status.
- Create a tasklist mirroring it, one task per ID. That list is what "go one by one" walks.

## 5. One item at a time

For each item, in list order, with the user in the loop:

1. **Explain** — restate the reviewer's point plainly: what, where (`file:line`), why they flagged it.
2. **Verify in code** — actually read the code and state your finding: confirmed / not-reproduced / partial, with the
   evidence. Reviewers and agent-drafted notes are sometimes wrong; don't accept on faith.
3. **Your take** — recommend a direction: fix here / defer (post-milestone) / follow-up ticket / won't-fix, with a
   one-line why.
4. **Ask** — where does the user want to take it? Don't implement until you've settled it together.

Once you've settled on a solution **with the user**:

05. **Implement** the agreed change — nothing more than was agreed.
06. **Comment pass** — run the `comment-discipline` skill over the diff.
07. **Self-review** — reread your change as a diff; confirm it does only what was agreed and that the project's validate
    target is green.
08. **Place it** — ask: new commit or `--fixup`?
    - New commit: write the message per the repo's commit rules.
    - Fixup: find the right target — the commit that introduced or last-touched those lines (`git log -- <file>`,
      `git blame`). Cite it by **subject line**, never SHA (the branch is being rewritten; SHAs go stale on every
      autosquash). Offer to create `git commit --fixup=<target>`; on confirm, create it and immediately trial-fold to
      prove it lands clean: `GIT_SEQUENCE_EDITOR=true GIT_EDITOR=true git rebase -i --autosquash <target>~1`. If it
      conflicts, `git rebase --abort`, say so, and pivot back to a standalone commit (or ask). Never force a conflicting
      fold.
    - Every git-state change happens only on the user's explicit go for that action: offer, wait, act.
09. **Reply** — offer to draft the thread reply. Show the draft in chat; post only when the user confirms (inline
    replies via the `gitlab-mr-inline-comments` skill). Cite the fix commit by subject line in backticks, and append the
    attribution footer your instructions require (`*Written by Claude, acked by <you>.*`). Never resolve the thread —
    resolution is the reviewer's signal, not the author's.
10. **Next** — only now move to the following item.

## Guardrails — never

- Never commit, amend, rebase, fold, push, post any MR note, or resolve any thread without an explicit go for that
  specific action. "Offer → confirm → act" is the pattern; a yes on one item is not a standing yes for the next.
- Never cite a commit by SHA in a reply while the branch is being rewritten — subject line in backticks.
- Never accept a finding without reading the code — a plausible note can still be wrong.
- Never batch items. One item fully closed before the next keeps each decision clean and reviewable.
