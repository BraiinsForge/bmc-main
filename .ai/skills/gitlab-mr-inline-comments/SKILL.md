---
name: gitlab-mr-inline-comments
description: Anchor a review comment to a specific line or hunk of a GitLab MR diff, and post MR notes, replies, and thread resolutions through `glab api`. Covers the `position` payload, the silent-drop pitfall where a bad position is demoted to a general comment behind an HTTP 200, and the mandatory verification step. Triggers on "post inline comments on the MR diff", "leave review comments on these lines", "post review notes on the diff", "post via glab CLI", or any request to anchor a comment to a specific line/hunk.
---

# GitLab MR line-anchored comments

Posting MR comments with `glab api`, and the mechanics of anchoring one to a diff line.

Line anchoring is the reason this skill exists — no GitLab MCP server currently exposes per-line diff comments, so those
go through `glab` whichever client the task started on. The other recipes here work equally well through a connected MCP
server: top-level notes, thread replies, resolutions. See `gitlab-mr-reply` for picking a client, then use the one you
picked.

The payload rules below are about GitLab's API, not about `glab`. They hold no matter what sends the request.

## The critical pitfall (inline comments)

**The GitLab API silently drops a malformed `position` and creates a general (non-inline) comment — while still
returning HTTP 200 with a success body.** `glab` will report success. The only way to know the comment landed inline is
to inspect the response and confirm the note came back with `type: "DiffNote"` and a populated `position.new_path` /
`position.new_line`.

Every posted inline comment MUST be verified this way. Do not batch-post without parsing the response for each call.

## Auth

`glab` uses `GITLAB_TOKEN` from the environment, or a prior `glab auth login`. Don't pass tokens on the command line.

## Reads

The MR metadata and diff can come from either client: a connected MCP server's discovered tool names, or the `glab api`
calls the examples below use. `gitlab-mr-reply` covers the one-time check for which you have.

When no project-lookup call is available, derive `project_id` from `git remote -v` and use the URL path form.

## Required context to gather first

Items 1–3 all come out of one merge-request read, so fetch it once:

```bash
glab api "projects/<group>%2F<project>/merge_requests/<iid>" | jq '{project_id, iid, diff_refs}'
```

1. **Project ID** — numeric, or the URL-encoded path (`bos%2Fbmc-main`). Numeric is safer in `glab api` paths, but the
   path form works and needs no lookup.

2. **MR IID** — from the URL, or by `source_branch=<branch>` when you only know the branch.

3. **Diff refs** — `diff_refs.{base_sha, start_sha, head_sha}` from that same response. All three are required in every
   `position` payload.

4. **New file paths and target line numbers** — read the MR diff and count line numbers in the *new* file. Don't
   eyeball; count from hunk headers `@@ -X,N +Y,M @@`.

   ```bash
   glab api "projects/<project_id>/merge_requests/<iid>/diffs?per_page=50" | jq -r '.[] | "### \(.new_path)\n\(.diff)"'
   ```

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

## General top-level note

A note with no `position` — the whole-MR comment:

```bash
glab api --method POST -H "Content-Type: application/json" \
  "projects/<project_id>/merge_requests/<mr_iid>/notes" \
  --input /tmp/mr-<iid>-note.json
```

Body JSON is `{"body": "..."}`. No silent-drop failure mode here — no verification needed beyond confirming HTTP 200.

Prefer this `glab api .../notes --input <file>` form over `glab`'s porcelain note subcommands: it's stable across glab
versions and carries the body as a file, so backticks/apostrophes never have to survive the shell. If you do reach for a
porcelain subcommand, check `glab <cmd> --help` first — its flags and deprecations drift between versions.

## Thread reply

Appending to an existing discussion rather than starting one:

```bash
glab api --method POST -H "Content-Type: application/json" \
  "projects/<project_id>/merge_requests/<mr_iid>/discussions/<discussion_id>/notes" \
  --input /tmp/mr-<iid>-reply.json
```

Where `<discussion_id>` is the hex thread ID and the JSON is `{"body": "..."}`.

## Resolve a thread

```bash
glab api --method PUT \
  "projects/<project_id>/merge_requests/<mr_iid>/discussions/<discussion_id>?resolved=true"
```

## Comment body conventions

These are about the comment, so they hold whichever client posts it. The full set lives in the `gitlab-mr-reply` skill;
summary:

- **Mirror the comment author's language.** Default to English for fresh top-level notes.
- **No commit hashes.** SHAs become 404s after fixup/autosquash. Describe the change semantically.
- **Footer:** `_drafted by Claude, reviewed by <name> before posting._` — `<name>` from `git config user.name`. Only add
  if the human actually reviewed before posting.
- **Draft-first.** Show the body in chat, wait for ack. Never auto-post.
- GFM markdown renders. Backticks, code fences, bold, lists all work.
- Keep each inline comment focused on ONE issue at ONE location. Cross-reference (`see :65`) rather than clubbing
  concerns.
- Include the *why*, not just "fix this".

## Hard rules — never

- Never `curl` the GitLab host by hand. Both clients handle auth and pagination; a raw `curl` puts a token in the
  transcript and silently mis-encodes bodies.
- Never assume a `glab` porcelain subcommand's flags are stable — prefer the `glab api` forms above, and check
  `glab <cmd> --help` before relying on one.
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
