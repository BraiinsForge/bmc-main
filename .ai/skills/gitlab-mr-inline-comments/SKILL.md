---
name: gitlab-mr-inline-comments
description: Use glab CLI for GitLab MR operations the MCP tools can't perform — primarily inline (per-line) diff comments, and as a fallback transport for general top-level notes or thread replies when the MCP path is unavailable. Triggers on "post inline comments on the MR diff", "leave review comments on these lines", "post review notes on the diff", "post via glab CLI", or any request to anchor a comment to a specific line/hunk.
---

# GitLab MR posting via glab CLI

End-to-end pipeline for posting MR comments via `glab api` when the connected GitLab MCP tools cannot do the operation.
Default to the MCP path (see the `gitlab-mr-reply` skill) — this skill is the fallback transport, primarily for inline
diff comments which the MCP proxy does not expose at the time of writing.

The connected proxy may prepend transport-specific prefixes to GitLab tool names. Discover the actual tool IDs first; do
not assume a fixed `mcp__mcp-proxy__...` prefix.

## When to use this skill vs the MCP one

| Operation                                 | Tool                                    |
| ----------------------------------------- | --------------------------------------- |
| Top-level MR note                         | discovered MCP create-note tool         |
| Reply to an existing thread               | discovered MCP discussion-reply tool    |
| Resolve a thread                          | discovered MCP resolve-thread tool      |
| Edit a posted note                        | discovered MCP update-note tool         |
| **Inline (per-line) comment on the diff** | **glab CLI (this skill)**               |
| MCP server unavailable / proxy down       | glab CLI (this skill, fallback section) |

If the operation has an MCP equivalent and the MCP server works, use it. Reach for glab CLI only when MCP genuinely
cannot help. Never `curl` the GitLab host directly.

## The critical pitfall (inline comments)

**The GitLab API silently drops a malformed `position` and creates a general (non-inline) comment — while still
returning HTTP 200 with a success body.** `glab` will report success. The only way to know the comment landed inline is
to inspect the response and confirm the note came back with `type: "DiffNote"` and a populated `position.new_path` /
`position.new_line`.

Every posted inline comment MUST be verified this way. Do not batch-post without parsing the response for each call.

## Auth

`glab` uses `GITLAB_TOKEN` from the environment, or a prior `glab auth login`. Don't pass tokens on the command line.

## Required tool discovery (read-side MCP context)

Before the first GitLab read call, discover the connected tool names you have available:

```
ToolSearch query="gitlab project merge request diffs discussions notes create reply resolve update"
```

Bind the discovered names for:

- project lookup (optional)
- get merge request
- get merge request diffs
- list discussions / notes (optional cleanup lookup)

Use those discovered names below. If project lookup is not available, derive `project_id` from `git remote -v` and use
the URL path form.

## Required context to gather first

1. **Project ID** — numeric (if the discovered project-lookup tool can resolve it) or URL-encoded path. Numeric is safer
   for `glab api` paths, but path form works.
2. **MR IID** — from the discovered get-merge-request tool with `project_id` and `source_branch=<branch>`.
3. **Diff refs** — from that same get-merge-request response, read `diff_refs.{base_sha, start_sha, head_sha}`.
4. **New file paths and target line numbers** — read the MR diff via the discovered get-merge-request-diffs tool and
   count line numbers in the *new* file. Don't eyeball; count from hunk headers `@@ -X,N +Y,M @@`.

## Inline comments — line classification

For each line you want to anchor on, classify it against the diff:

| Line type                 | What to send                                             |
| ------------------------- | -------------------------------------------------------- |
| **Added** (`+` in diff)   | `new_path` + `new_line` only                             |
| **Removed** (`-` in diff) | `old_path` + `old_line` only                             |
| **Context** (unchanged)   | BOTH `new_path` + `new_line` AND `old_path` + `old_line` |

Prefer anchoring on added lines — context-line position errors are the most common silent-drop cause.

## Inline comment — exact recipe

Write each payload to a temp file (avoids shell-escape hell with backticks and embedded code in the body), then call
`glab api`:

````json
// /tmp/mr-<iid>-comment-<n>.json
{
  "body": "markdown body with `inline code` and ```fenced blocks``` as needed",
  "position": {
    "base_sha": "<base_sha>",
    "start_sha": "<start_sha>",
    "head_sha": "<head_sha>",
    "position_type": "text",
    "new_path": "<path/as/shown/in/diff>",
    "new_line": <line number>
  }
}
````

```bash
glab api --method POST \
  -H "Content-Type: application/json" \
  "projects/<project_id>/merge_requests/<mr_iid>/discussions" \
  --input /tmp/mr-<iid>-comment-<n>.json
```

The `Content-Type: application/json` header is mandatory — without it the body silently form-encodes and the `position`
drops.

## Verification (non-negotiable for inline)

Parse the response for every single inline comment. A good one-liner:

```bash
glab api --method POST -H "Content-Type: application/json" \
  "projects/<project_id>/merge_requests/<mr_iid>/discussions" \
  --input /tmp/mr-<iid>-comment-<n>.json 2>&1 \
  | python3 -c "import json,sys; d=json.load(sys.stdin); n=d['notes'][0]; print('id:', n['id'], '| type:', n['type'], '| new_path:', n['position'] and n['position']['new_path'], '| new_line:', n['position'] and n['position']['new_line'])"
```

Success looks like:

```
id: <id> | type: DiffNote | new_path: <path> | new_line: <line>
```

Failure modes:

- `type: Note` instead of `DiffNote` — `position` dropped, it's now a general comment. Delete it and re-post after
  fixing the position:
  ```
  glab api --method DELETE projects/<project_id>/merge_requests/<mr_iid>/notes/<note_id>
  ```
- `position: null` in response — same as above.
- `new_line` / `new_path` differ from what you sent — API remapped silently. Investigate the hunk; line is probably
  off-by-one against the new file.

## Test one first, then batch

When posting multiple inline comments, **post one and verify it landed correctly** before firing the rest. The common
failure (wrong sha, wrong path casing, line off the hunk) reveals itself on the first attempt; you avoid spamming the MR
with botched general comments that need cleanup.

## General top-level note via glab CLI (fallback only)

When MCP is unavailable, the equivalent of the create-note tool:

```bash
glab api --method POST -H "Content-Type: application/json" \
  "projects/<project_id>/merge_requests/<mr_iid>/notes" \
  --input /tmp/mr-<iid>-note.json
```

Body JSON is `{"body": "..."}`. No silent-drop failure mode here — no verification needed beyond confirming HTTP 200.

## Thread reply via glab CLI (fallback only)

When MCP is unavailable, the equivalent of the discussion-reply tool:

```bash
glab api --method POST -H "Content-Type: application/json" \
  "projects/<project_id>/merge_requests/<mr_iid>/discussions/<discussion_id>/notes" \
  --input /tmp/mr-<iid>-reply.json
```

Where `<discussion_id>` is the hex thread ID and the JSON is `{"body": "..."}`.

## Resolve a thread via glab CLI (fallback only)

```bash
glab api --method PUT \
  "projects/<project_id>/merge_requests/<mr_iid>/discussions/<discussion_id>?resolved=true"
```

## Comment body conventions

These rules apply equally to MCP-posted and glab-posted comments. The full set lives in the `gitlab-mr-reply` skill;
summary:

- **Mirror the comment author's language.** Default to English for fresh top-level notes.
- **No commit hashes.** SHAs become 404s after fixup/autosquash. Describe the change semantically.
- **Footer:** `_drafted by Claude, reviewed by <name> before posting._` — `<name>` from `git config user.name`. Only add
  if the human actually reviewed before posting.
- **Draft-first.** Show the body in chat, wait for ack. Never auto-post — this applies to glab CLI just as it does to
  MCP.
- GFM markdown renders. Backticks, code fences, bold, lists all work.
- Keep each inline comment focused on ONE issue at ONE location. Cross-reference (`see :65`) rather than clubbing
  concerns.
- Include the *why*, not just "fix this".

## Hard rules — never

- Never `curl` the GitLab host. Use `glab api` (handles auth) or the MCP tools.
- Never auto-post a reply.
- Never include commit hashes in a reply.
- Never amend or force-push to "fix" a typo. Edit via
  `glab api --method PUT projects/<id>/merge_requests/<iid>/notes/<note_id>` or delete + re-post.
- Never `git push` after posting. The user handles `git push` themselves.

## Useful endpoints

- `GET projects/:id/merge_requests/:iid` — diff refs live in `diff_refs`
- `GET projects/:id/merge_requests/:iid/diffs` — per-file hunks
- `GET projects/:id/merge_requests/:iid/discussions` — list (find duplicates or IDs to delete)
- `POST projects/:id/merge_requests/:iid/discussions` — inline comment (this skill's main target)
- `POST projects/:id/merge_requests/:iid/notes` — top-level note (fallback)
- `POST projects/:id/merge_requests/:iid/discussions/:discussion_id/notes` — thread reply (fallback)
- `PUT projects/:id/merge_requests/:iid/notes/:note_id` — edit a note
- `PUT projects/:id/merge_requests/:iid/discussions/:discussion_id?resolved=true` — resolve thread (fallback)
- `DELETE projects/:id/merge_requests/:iid/notes/:note_id` — delete a botched note
