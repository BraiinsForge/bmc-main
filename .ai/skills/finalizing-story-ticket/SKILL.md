---
name: finalizing-story-ticket
description: Use when finalizing a Story-type ticket — the addendum to `finalizing-ticket` that captures the user-facing feature in `docs/stories/`. Triggers on phrases like "finalize the story", "submit the story for review", "add the user story doc", "document this feature", "it's a story ticket", or when `finalizing-ticket` detects the Jira issue type is Story. Covers the `docs/stories/` document shape and the README index update.
---

# Finalizing a Story ticket

A Story ships user-visible behavior, so finishing it includes documenting *what the user gets* — not just the code. That
documentation lives in `docs/stories/` and is a permanent, user-facing deliverable. This skill is the story-specific
step of `finalizing-ticket`; run the rest of that checklist too.

## Story doc vs scratch devlog — keep them straight

- `docs/devlogs/BDK-<ticket>-<slug>/` — **scratch**. How the work got built, stage by stage. Deleted when the ticket is
  done; never part of the review.
- `docs/stories/<feature>.md` — **permanent**. What the feature does for the user, in their terms. Lands in the MR and
  stays in the tree.

If the only artifact is a devlog, the story is not finalized.

## Add or update the feature document

Create `docs/stories/<feature>.md` (or extend the existing one when the ticket grows a feature already documented).
Match the shape of the existing docs (see `docs/stories/combined-scene.md`):

```markdown
# Feature Title

One- or two-sentence framing of the feature in user terms — what it lets the user do and why.

## User stories

### Short story name

> As a <role>, I want <capability> so that <benefit>.

- Concrete, user-observable behavior point.
- Another behavior point — what the device actually does, not how it's implemented.

## Constraints

- User-facing limits and compatibility rules that scope the feature.
```

Write for the user, not the implementer: behavior and guarantees, no module names, types, or code paths. One `###`
sub-story per distinct capability, each with its `> As a …` blockquote and acceptance bullets.

## Update the index

`docs/stories/README.md` is the feature index. Add the new feature under `## Features` as a linked heading with a short
summary, mirroring the existing entries. Several feature areas are present as `*Not yet documented.*` placeholders —
when a Story fills one of those in, replace the placeholder rather than appending a duplicate heading.

## Format the docs, then validate

Right after writing or editing `docs/stories/<feature>.md` and `docs/stories/README.md`, run the formatter over the tree
— `nix fmt` (or `just validate`, which runs it) — before staging. Markdown goes through the workspace formatter, which
rewraps prose and normalizes lists; hand-written line breaks will otherwise drift from the formatted result and the
`content` check / CI will bounce it. Do not skip this because "it's only docs" — the formatter is not optional for
markdown.

Then finish with the full `just validate` (the `content` check plus `nix fmt`) so the docs land already-formatted; see
`repo-build-workflow`.

## Hard rules — never

- Never finalize a Story with only a devlog — the `docs/stories/` entry is required and the devlog is deleted.
- Never stage or commit a story doc without running the markdown formatter (`nix fmt` / `just validate`) over it first.
- Never describe the feature in implementation terms (modules, types, functions) in a story doc; it is user-facing.
- Never add a new README heading for a feature that already has a `*Not yet documented.*` placeholder — replace it.
- Never leave `docs/stories/README.md` out of date after adding a story document.
