---
name: gitlab-mr-reply
description: Use when replying to a comment on a GitLab MR, drafting a response to a reviewer, reloading MR data, or resolving discussion threads. Triggers on phrases like "let's answer the MR comment", "reply to this comment", "reload the MR data", "close those threads", "look at the MR review", or when a screenshot of a reviewer's comment is shared and a response is requested.
---

# GitLab MR review-reply procedure

End-to-end pipeline for reading and replying to MR comments. Opinionated about how replies are drafted, reviewed, and
posted — the rules below are not optional, whichever client carries them.

## Pick a client (once, up front)

Every operation here works through either a connected GitLab MCP server or `glab`. Establish which one you have before
step 1, then stay on it — don't re-check per operation:

```
ToolSearch query="+gitlab merge request discussions"
```

- **Hits** → use the discovered tool names verbatim; proxies prepend their own prefixes, so never hardcode `mcp__…`.
- **No hits** → `glab api`, per the `gitlab-mr-inline-comments` skill.

Neither is a downgrade — both are authenticated and complete, and differ only in which operations they expose. If the
client you picked can't do one step, switch that step to the other rather than inventing a tool name; line-anchored
comments are the usual gap.

## Procedure

### 1. Identify the MR

- `git remote -v` to read the remote URL.
- Derive `project_id` by stripping protocol/user and `.git` suffix from the path component (form: `<group>/<project>`).
- Resolve that path to a numeric project ID if a project-lookup call is available; otherwise keep the path form,
  URL-encoded (`bos%2Fbmc-main`).
- **Get the merge request** by `project_id` + `source_branch=<current branch>` → capture `iid`, `web_url`, `state`,
  `head_sha`. On `glab`: `glab api "projects/<project_id>/merge_requests?source_branch=<branch>"`.

### 2. Fetch discussions

**List the MR's discussions** with `per_page=100`. On `glab`:
`glab api "projects/<project_id>/merge_requests/<iid>/discussions?per_page=100"`.

Response-shape gotchas — the first applies to MCP responses only, the rest to both:

- An MCP payload is `{items: [...], pagination: {...}}`, not a flat array; top-level access goes through `.items`.
  `glab api` returns the flat array the REST API documents.
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

- **Top-level MR note** when the message is summary-level or addresses several threads in one go.
- **Thread reply**, against `discussion_id=<thread id>`, when the response is line-bound and belongs in the original
  thread.

Both surfaces, and the resolve and edit calls below, exist in either client — the discovered tool names if you have a
server, otherwise the `glab api` recipes in `gitlab-mr-inline-comments`.

When the user has agreed a discussion can close, **resolve** each closed thread after its post. Do not resolve threads
the user hasn't ack'd.

To fix a typo in a posted reply, **edit the note** — never amend or force-push to "fix" an already-published comment.

### 8. Cadence — per-task, not batched

When working through a queue of MR threads, post the reply on each thread the moment that item is done — locally
validated, fixup committed — *before* starting the next item. Don't batch all replies into a single dump at the end.

The reason: batching loses correlation between threads and fixup commits, makes the reviewer's triage harder, and leaves
threads silent for hours while you grind through the queue. A steady "this one's handled" stream signals progress
without the reviewer having to chase.

If a fix has both an inline thread and a file-summary mirror thread containing the same content, reply only on the
inline canonical — the mirror is bot-generated and a second reply just clutters.

## Hard rules — never

- Never `curl` the GitLab host by hand. Both clients handle auth and pagination; a raw `curl` puts a token in the
  transcript and silently mis-encodes bodies.
- Never `git push` after posting. The user handles `git push` themselves.
- Never auto-post a reply.
- Never include commit hashes in a reply.
- Never amend or force-push to "fix" a typo in a posted reply.

## Line-anchored replies

No GitLab MCP server currently exposes inline (per-line) diff comments, so those go through `glab api` regardless of
which client you picked. The `gitlab-mr-inline-comments` skill covers the position payload and its verification step.

Everything above still applies there: the drafting rules in section 5 (language mirror, no SHAs, footer) and the
draft-first rule in section 6 are about the reply, not the transport.
