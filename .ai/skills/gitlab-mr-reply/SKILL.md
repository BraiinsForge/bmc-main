---
name: gitlab-mr-reply
description: Use when replying to a comment on a GitLab MR, drafting a response to a reviewer, reloading MR data through MCP, or resolving discussion threads. Triggers on phrases like "let's answer the MR comment", "reply to this comment", "reload the MR data", "close those threads", "look at the MR review", or when a screenshot of a reviewer's comment is shared and a response is requested.
---

# GitLab MR review-reply procedure

End-to-end pipeline for reading and replying to MR comments via the connected GitLab MCP tools. Opinionated about how
replies are drafted, reviewed, and posted — the rules below are not optional.

## Required tool discovery (load first)

The GitLab MCP tools are deferred, and the connected proxy may prepend transport-specific prefixes to every tool name.
Do **not** hardcode exact IDs like `mcp__mcp-proxy__...` up front. Discover the connected tool names first, then use the
discovered names consistently for the rest of the workflow.

Use tool search to find the GitLab MR read/mutation capabilities you need:

```
ToolSearch query="gitlab merge request get discussions notes create reply resolve update project"
```

Bind the discovered names for these capabilities:

- project lookup (optional, only if available)
- get merge request
- list MR discussions
- list MR notes
- create top-level MR note
- create discussion reply
- resolve discussion thread
- update an existing note

If some mutation capability is missing from the discovered set, switch that part of the workflow to the
`gitlab-mr-inline-comments` skill's `glab api` fallback instead of inventing a tool name.

## Procedure

### 1. Identify the MR

- `git remote -v` to read the remote URL.
- Derive `project_id` by stripping protocol/user and `.git` suffix from the path component (form: `<group>/<project>`).
- If the discovered project-lookup tool exists, you may resolve that path to a numeric project ID; otherwise keep using
  the path form.
- Call the discovered get-merge-request tool with `project_id` and `source_branch=<current branch>` → capture `iid`,
  `web_url`, `state`, `head_sha`.

### 2. Fetch discussions

Call the discovered MR-discussions tool with `project_id`, `merge_request_iid`, `per_page=100`.

Response-shape gotchas:

- The payload is `{items: [...], pagination: {...}}`, not a flat array. Top-level access goes through `.items`.
- The body almost always exceeds the token cap and is dumped to a file. Read that file with `jq` rather than
  re-requesting smaller pages — pagination is by thread count, not byte size, so smaller `per_page` rarely helps.
- If `pagination.x_total_pages > 1`, paginate across pages.

### 3. Triage notes

For each `items[*]`:

- Skip notes where `notes[*].system: true` — these are commit refs, branch updates, "added N commits" auto-posts. Pure
  noise.
- Real review content is in notes with `system: false`.
- Capture per relevant thread: thread `id` (the hex string), each note's `author.username`, `body`, `created_at`,
  `updated_at`, `position.{new_path,new_line}`, plus thread-level `resolvable` and `resolved`.

### 4. Read the referenced code

For every thread you'll reply to, open the file at `position.new_path:position.new_line` and read enough surrounding
context to understand the current state. **Replies must be grounded in the code on disk right now**, not a paraphrase of
the comment text. The reviewer's last note may be older than the latest commit; verify before agreeing something is
"fixed".

### 5. Drafting rules

- **Mirror the comment author's language.** Match whichever language the reviewer wrote in. Default to English when
  unclear or for a fresh top-level note.

- **No commit hashes.** Never reference SHAs in MR replies. Fixup/autosquash rewrites them and a hash becomes a 404 or
  stale link the moment the branch is rebased. Describe the change semantically (e.g. "in the latest push", "the X
  branch now calls Y", "hoisted Z out"). File paths and line numbers are fine; SHAs are not.

- **Trailing pointer note, not an essay.** A reply addressing a single thread is one or two sentences pointing at where
  the change landed — not a bulleted recap of the diff, not a restatement of what was decided in conversation, not a
  paste of the new content. A short quip is fine. If the user corrects the tone on a posted reply, don't try to rewrite
  it; assume they've already edited it themselves.

- **Push back when warranted.** Don't be a yes-man. If the reviewer asks to close a thread but the code still has a real
  issue, say so. Flag fragile approaches.

- **Reviewer footer.** Every posted reply gets:

  > _drafted by Claude, reviewed by \<name> before posting._

  Resolve `<name>` from `git config user.name` at post time. Do **not** add the footer if a reply was posted without
  prior human review — the footer is a truth claim, not boilerplate.

### 6. Draft-first (mandatory, non-negotiable)

Show the full proposed body in chat as a quoted/codeblock draft. Wait for explicit ack. **Never auto-post a reply.**

The reviewer is interacting with what they think is the human author. They deserve to know which parts are
agent-drafted, and the human deserves a chance to catch tone, accuracy, or scope issues before words hit a public
thread.

If the request says "post it" without showing intent to review, present the draft anyway — one extra round trip is
cheap; an embarrassing public reply is not.

### 7. Posting

Pick the right surface:

- **Top-level MR note** (the discovered create-note tool) when the message is summary-level or addresses multiple
  threads in one go.
- **Thread reply** (the discovered create-discussion-reply tool with `discussion_id=<thread id>`) when the response is
  line-bound and belongs in the original thread.

When the user has agreed the discussion can close, follow each post with the discovered resolve-thread tool for each
closed thread. Do not resolve threads the user hasn't ack'd.

To fix a typo in a posted reply, use the discovered update-note tool — never amend or force-push to "fix" an
already-published comment.

### 8. Cadence — per-task, not batched

When working through a queue of MR threads, post the reply on each thread the moment that item is done — locally
validated, fixup committed — *before* starting the next item. Don't batch all replies into a single dump at the end.

The reason: batching loses correlation between threads and fixup commits, makes the reviewer's triage harder, and leaves
threads silent for hours while you grind through the queue. A steady "this one's handled" stream signals progress
without the reviewer having to chase.

If a fix has both an inline thread and a file-summary mirror thread containing the same content, reply only on the
inline canonical — the mirror is bot-generated and a second reply just clutters.

## Hard rules — never

- Never `curl` the GitLab host. The MCP tools are authenticated and structured.
- Never `git push` after posting. The user handles `git push` themselves.
- Never auto-post a reply.
- Never include commit hashes in a reply.
- Never amend or force-push to "fix" a typo in a posted reply.

## When MCP can't do the operation

The MCP proxy does not currently expose inline (per-line) diff comments, and may be unavailable in some sessions. For
those cases switch to the `gitlab-mr-inline-comments` skill, which uses `glab api` as the transport. The drafting rules
in section 5 (language mirror, no SHAs, footer) and the draft-first rule in section 6 apply there too — only the
transport changes.
