---
name: jira-ticket-writing
description: Use when creating or editing BDK Jira tickets, choosing their type or parent, or writing their descriptions. Triggers on requests to create a Jira ticket, file follow-up tickets, update a ticket description, or use `acli jira workitem`.
---

# Writing a Jira ticket

## Choose the type and parent first

- **Bug:** Something that should already work is wrong, including an internal failure with no visible UI symptom. Do not
  reclassify it as a Task merely because it is technical. Put it under the Bug-tracking epic for the release that
  **introduced** the defect, not the release planned to fix it or the feature epic where it was found. Look up the
  relevant release and epic in Jira; their names and keys change. If the introducing release cannot be established,
  investigate or ask rather than guessing a parent. If the bug is part of an ongoing effort, not yet released, put it
  under the epic for that effort.
- **Story:** A new user-facing capability or a design deliverable that someone will implement. A designer receives
  requirements through a Story ticket. The stories should define the full user flow, especially for cases where a design
  will be necessary.
- **Task:** Investigation, decision, or bounded technical work that is neither a defect nor a user-facing Story. The
  acceptance criteria for an investigation should name the decision or follow-up tickets it must produce.
- **Epic:** An outcome spanning multiple Stories or Tasks. Use the relevant existing epic for child work rather than
  creating a duplicate.

Every Story, Task, and Bug must have a parent Epic so the work can be found with its related work. Check existing Epics
and choose the most specific one that fits the outcome before creating the ticket. Epics themselves are the grouping
level in the current BDK hierarchy and may have no parent.

Use [BDK-221, Deck Side Technology Debt](https://braiins.atlassian.net/browse/BDK-221) only for non-user-visible
technology debt that has no better parent. A technical Task or Story can still belong under a feature or tooling Epic.
An internal Bug still follows the Bug-parent rule above; lack of a UI symptom does not make it technology debt.

## Choosing fix version

The fix version denotes the release a change goes into. For work that is going to happen very soon, or needs to be fixed
in the next release, choose that release before reporting the ticket complete. If the target release is not yet known,
leave Fix version unset and fill it in once the release is known, by the time the ticket is done.

Create the ticket with acli, then add a known Fix version with jira-cli. `fixVersions` is a Jira system field, not a
custom field. `acli jira workitem edit` has no Fix version flag and rejects both `fixVersions` and
`additionalAttributes` in `--from-json`:

```bash
jira issue edit PROJ-123 --fix-version "26.10" --no-input
```

`--fix-version` appends. When changing the planned release, remove the stale version with a minus-prefixed name:

```bash
jira issue edit PROJ-123 --fix-version -26.10 --fix-version 26.11 --no-input
```

List existing release names with `jira release list -p BDK`. Verify the saved version list with
`acli jira workitem view --json`, including that an old target was removed; do not silently leave Fix version unset when
the release is known.

## Ticket content

Use a specific summary and a short description of the problem or intended outcome. Include the context, scope, and
expected behavior needed to act on the ticket; do not pad it with implementation guesses. State dependencies and link
related tickets when they affect sequencing.

Every ticket needs a literal `## Acceptance criteria` heading followed by real Markdown bullet-list items (`- ...`).
Each item should describe a verifiable outcome. For a Bug, cover the corrected behavior and a regression check. For an
investigation, require a documented conclusion and concrete follow-up work where needed. For a design Story, state what
the designer must produce and what information they need. Avoid criteria that merely repeat implementation steps.

Use explicit Markdown links for every URL and Jira issue reference, such as
`[Home Assistant WebSocket API](https://developers.home-assistant.io/docs/api/websocket/)` and
`[BDK-201](https://braiins.atlassian.net/browse/BDK-201)`. `md_to_adf.py` emits bare URLs and bare `BDK-123` text as
plain text, with no link. Use a `## References` heading when references warrant their own section. After submission,
inspect the raw stored description for heading, bullet-list, and link nodes; correct any literal Markdown or unlinked
references.

## Writing Stories

Begin a Story with a `## User story` heading and a statement from the perspective of the person who will use the result:

> As a [specific user], I want [observable capability], so that [benefit].

For example: “As a device user, I want to see whether my widget is connected, so that I know if its data is current.”
Choose the actor based on who uses the result. Many features in this codebase serve both Deck and miners such as BMM101,
so do not default to “Deck user” or Deck-only wording. Use a narrower actor when the feature truly applies only to that
product or role.

Describe the user's expected flow after the statement: how they reach the feature, configure or use it, and see success
or failure, as relevant. Keep implementation choices out of this section. If technical direction is needed, put it under
a separate `## Technical implementation` heading. A design Story still states the intended user experience; its
acceptance criteria specify the design deliverables.

## Convert, never paste

Jira Cloud stores rich text as Atlassian Document Format. Markdown handed to a description field is stored **literally**
— readers see `## Goal` and `**bold**` as characters — and Atlassian publishes no Markdown converter, only ADF builders.

Write the body as Markdown in a temporary file under `.tmp/jira/`, then convert:

```bash
just md-to-adf .tmp/jira/PROJ-123.md --skip-title > .tmp/jira/PROJ-123.json

# onto an issue that exists
acli jira workitem edit --key PROJ-123 --description-file .tmp/jira/PROJ-123.json --yes

# or as a new design Story under an epic, the summary carrying what `--skip-title` dropped
acli jira workitem create --project PROJ --type Story --parent PROJ-1 \
  --summary "Design the Widget for SOMEDEVICE" --description-file .tmp/jira/PROJ-123.json
```

A `PreToolUse` hook refuses `--description`/`--from-file` on either subcommand: those take the text inline, so markdown
reaches the field unconverted and nothing errors until a reader opens the issue.

`--skip-title` drops a leading `#` heading, for when the title is already the Jira summary. Never hand-author ADF: it is
verbose, easy to get subtly wrong, and drifts from the markdown it came from.

Drafting does not authorize creation. Submit only when the user has asked to create the ticket or approved the draft.
After creation, verify the issue type, parent, description structure, and links in Jira using acli's JSON output; report
the issue key. For example:

```bash
acli jira workitem view PROJ-123 --json --fields key,issuetype,parent,description,fixVersions \
  | jq '{type:.fields.issuetype.name, parent:.fields.parent.key, fixVersions:[(.fields.fixVersions // [])[].name], blocks:[(.fields.description.content // [])[].type], links:[.. | objects | select(.type? == "link") | .attrs.href]}'
```

Check that `blocks` contains `heading` and `bulletList`, `links` contains each intended URL and issue link, and
`fixVersions` contains the release when one was required. An Epic may have a null parent.

`acli` cannot upload attachments — `create`, `edit` and `--from-json` have no attachment field, and
`acli jira workitem attachment` offers only `list` and `delete`. Images have to be attached by hand.

## Jira command-line clients

Use `acli jira workitem` to create, edit, and verify tickets. Its description-file option accepts the converted ADF, and
`view --json` exposes the stored fields needed for verification. Use jira-cli only for what acli cannot do: fields it
cannot edit, such as Fix version above, and raw API calls such as `jira api "users?search=<name>"`. Verify the result
with acli afterwards.

If acli is unavailable, the Atlassian MCP is the remaining sink that accepts ADF: pass the converter's JSON as
`description` with `contentFormat: "adf"`, since its default `markdown` format converts on Atlassian's side and skips
the fail-loud converter. Verify with `getJiraIssue` using `responseContentFormat: "adf"`. Do not fall back to jira-cli
for descriptions: it converts markdown itself and its README warns the conversion is lossy.
