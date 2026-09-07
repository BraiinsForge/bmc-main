---
name: jira-ticket-writing
description: Use when creating or editing a Jira work item — writing a ticket description, filing follow-up tickets under an epic, or updating an existing issue's body. Triggers on phrases like "create a Jira ticket", "file these as tickets", "update the ticket description", "write up the epic", or on any use of `acli jira workitem`.
---

# Writing a Jira ticket

## Convert, never paste

Jira Cloud stores rich text as Atlassian Document Format. Markdown handed to a description field is stored **literally**
— readers see `## Goal` and `**bold**` as characters — and Atlassian publishes no Markdown converter, only ADF builders.

Write the body as markdown in the repo, then convert:

```bash
just md-to-adf docs/devlogs/PROJ-123-example/ticket.md --skip-title > .tmp/jira-adf-PROJ-123.json

# onto an issue that exists
acli jira workitem edit --key PROJ-123 --description-file .tmp/jira-adf-PROJ-123.json --yes

# or as a new child of an epic, the summary carrying what `--skip-title` dropped
acli jira workitem create --project PROJ --type Task --parent PROJ-1 \
  --summary "Design the Widget for SOMEDEVICE" --description-file .tmp/jira-adf-PROJ-123.json
```

A `PreToolUse` hook refuses `--description`/`--from-file` on either subcommand: those take the text inline, so markdown
reaches the field unconverted and nothing errors until a reader opens the issue.

`--skip-title` drops a leading `#` heading, for when the title is already the Jira summary. Never hand-author ADF: it is
verbose, easy to get subtly wrong, and drifts from the markdown it came from.

`acli` cannot upload attachments — `create`, `edit` and `--from-json` have no attachment field, and
`acli jira workitem attachment` offers only `list` and `delete`. Images have to be attached by hand.

## Two clients, split by job

Neither Jira client covers the other's work, so both stay:

- **acli owns anything with a body.** `md_to_adf.py` raises on a construct it cannot map, rather than dropping it from a
  ticket, and a fail-loud converter needs a sink that takes ADF. acli is the only such sink driven from a shell, so one
  path serves the `just` recipe, the hook and every agent. The alternatives convert markdown themselves and are quiet
  about what they lose: jira-cli's README warns that "not all Atlassian nodes are translated properly at the moment
  which can cause formatting issues sometimes", and the Atlassian MCP takes markdown by default.
- **jira-cli owns the raw API and custom fields.** acli has no `api` passthrough:
  `auth board dashboard field filter project sprint workitem` is its whole Jira surface. So
  `jira api "users?search=<name>"` has no acli equivalent, and `finalizing-ticket` depends on exactly that call.

Don't consolidate onto one of them.
