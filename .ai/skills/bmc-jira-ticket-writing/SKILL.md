---
name: bmc-jira-ticket-writing
description: Use when creating or editing BDK Jira tickets, choosing their type or parent, or writing their descriptions. Triggers on requests to create a Jira ticket, file follow-up tickets, update a ticket description, set a fix version, or use the Atlassian MCP or `acli jira workitem`.
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

List the release names with the `getJiraProjectVersions` operation (`executeRead`, `projectIdOrKey: "BDK"`,
`status: ["unreleased"]`). Set the version with `editJiraIssue`; `fixVersions` is a system field passed in raw shape
under `additional_fields`:

```json
{ "fixVersions": [{ "name": "26.10" }] }
```

The list is set, not appended: send the full list, and `[]` to clear it. The edit response does not echo `fixVersions`;
verify with `getJiraIssue` using `view: "full"`, `fields: ["fixVersions"]` and `fieldsByKeys: true`, since the compact
view omits the field even when asked for it. Do not silently leave Fix version unset when the release is known.

When the MCP is not connected, do not reach for another tool to set the version: tell the user which release the ticket
should carry and ask them to connect the MCP or set the field themselves.

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
`[BDK-201](https://braiins.atlassian.net/browse/BDK-201)`. A bare `BDK-123` key is stored as plain text by both
submission paths; a bare URL becomes a smart-link card through the MCP and plain text through `md_to_adf.py`. Use a
`## References` heading when references warrant their own section. After submission, inspect the stored description for
heading, bullet-list, and link nodes; correct any literal Markdown or unlinked references.

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

## Submitting the description

Jira Cloud stores rich text as Atlassian Document Format. Write the body as Markdown in a file under `.tmp/jira/` and
keep to constructs both submission paths map: headings, paragraphs, bold, italic, inline code, fenced code, explicit
links, bullet and numbered lists, tables, blockquotes, rules.

**Atlassian MCP, primary.** `createJiraIssue` and `editJiraIssue` take the Markdown as `description` and convert it on
Atlassian's side. `contentFormat` accepts only `markdown` or `html`; there is no ADF input. Verified on a probe ticket:
headings, nested lists, task lists, blockquotes, fenced code with its language, tables with a header row, rules,
explicit links and strikethrough all arrive as proper nodes. Two silent losses: a hard break (two trailing spaces) stays
literal inside the paragraph, and an image reference becomes an external media node Jira does not render. Pass the
parent key as `parent` and the type as `issueType`.

**acli, secondary.** acli hands the description to Jira verbatim, so convert first; unconverted Markdown is stored
**literally** and readers see `## Goal` and `**bold**` as characters:

```bash
just md-to-adf .tmp/jira/PROJ-123.md --skip-title --output .tmp/jira/PROJ-123.json

# onto an issue that exists
acli jira workitem edit --key PROJ-123 --description-file .tmp/jira/PROJ-123.json --yes

# or as a new design Story under an epic, the summary carrying what `--skip-title` dropped
acli jira workitem create --project PROJ --type Story --parent PROJ-1 \
  --summary "Design the Widget for SOMEDEVICE" --description-file .tmp/jira/PROJ-123.json
```

A `PreToolUse` hook refuses `--description`/`--from-file` on either subcommand: those take the text inline, so markdown
reaches the field unconverted and nothing errors until a reader opens the issue.

`--skip-title` drops a leading `#` heading, for when the title is already the Jira summary. `md_to_adf.py` refuses a
construct it cannot map, such as an image, rather than dropping it, but emits strikethrough and task lists as literal
text. Never hand-author ADF: it is verbose, easy to get subtly wrong, and drifts from the markdown it came from.

Drafting does not authorize creation. Submit only when the user has asked to create the ticket or approved the draft.
After creation, verify the issue type, parent, description structure, and links in Jira; report the issue key.

Through the MCP, `getJiraIssue` with `responseContentFormat: "html"` renders the stored document: check for `<h2>`,
`<ul>` or `<ol>` and `<a href>` where headings, lists and links were intended, and for no literal `##` or `**`. The MCP
does not expose the raw ADF nodes; for those use acli:

```bash
acli jira workitem view PROJ-123 --json --fields key,issuetype,parent,description,fixVersions \
  | jq '{type:.fields.issuetype.name, parent:.fields.parent.key, fixVersions:[(.fields.fixVersions // [])[].name], blocks:[(.fields.description.content // [])[].type], links:[.. | objects | select(.type? == "link") | .attrs.href]}'
```

Check that `blocks` contains `heading` and `bulletList`, `links` contains each intended URL and issue link, and
`fixVersions` contains the release when one was required. An Epic may have a null parent.

`acli` cannot upload attachments — `create`, `edit` and `--from-json` have no attachment field, and
`acli jira workitem attachment` offers only `list` and `delete`. The MCP lists an `uploadAttachmentToJiraIssue`
operation, untested here. Otherwise images have to be attached by hand.

## Linking tickets

With acli, `--out` takes the ticket that is blocked and `--in` the one that blocks it. To record that BDK-1 blocks
BDK-2, run `acli jira workitem link create --out BDK-2 --in BDK-1 --type Blocks --yes`. Its success message prints the
pair the other way round ("BDK-2 Blocks BDK-1"), so do not take it as the result. Check the stored direction in Jira's
own wording instead:

```bash
acli jira workitem view BDK-1 --json --fields issuelinks \
  | jq -r '.fields.issuelinks[] | if .inwardIssue then "\(.type.inward) \(.inwardIssue.key)" else "\(.type.outward) \(.outwardIssue.key)" end'
```

A link stored the wrong way round is removed with `acli jira workitem link delete --id <id>`; the id is each entry's
`.id` in the same JSON.

## Jira clients

The Atlassian MCP is the primary client: `createJiraIssue`, `editJiraIssue` and `getJiraIssue` are primary tools; other
operations are found with `discover` and run through `executeRead` or `executeWrite`. Every call needs the `cloudId`
from `getAccessibleAtlassianResources`, fetched once per session, and calls draw on the organisation's Rovo credits. It
is a remote server the user connects once, then authorises in a session with `/mcp`:

```bash
claude mcp add --transport http --scope user atlassian https://mcp.atlassian.com/v2/mcp
```

Use acli when the MCP is not connected: `acli jira workitem` creates, edits and verifies tickets with the converted ADF
and `view --json`. It is not in the dev shell; run it as `nix run .#pkgs.acli -- jira …`, or open
`nix shell .#pkgs.acli` for a session.
