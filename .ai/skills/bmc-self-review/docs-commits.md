# Docs and commits checklist

## Docs and comments

- **Fix stale docs in the same branch.** Grep `docs/`, `.ai/` (the instructions and skills), the crate READMEs and doc
  comments for every symbol, behaviour, flag and path the diff renames, removes or changes. A doc the branch made wrong
  is the branch's to fix.
- **No overclaims.** A doc, a comment, a commit body or the MR description states only what the code does. "Gives up
  after ten attempts" needs code that stops after ten. "Never blocks" needs code that can't block.
- **One rationale, one place.** A why lives at the definition it explains, or in the one doc that owns the topic. Other
  places point there instead of restating it.
- **Comments** pass the `comment-discipline` skill. In particular:
  - a Jira key appears only in a `TODO(BDK-123): …`;
  - verification records ("verified on a Deck") belong in the MR description, never in a code comment or a devlog on the
    branch.
- **Rendered text isn't wrapped for one view.** A string another tool lays out, such as a JSON schema `description`,
  breaks only between sentences, never mid-sentence to fit the width the author happened to see.
- **No secrets.** The MR description, commit messages, docs and test fixtures carry no real session id, password, token
  or key.
- **Wording.** Docs call the software "the BMC application", never bare BMC or bmc. What ships with the profile
  (packages, language packs, the BMC application) is "the BMC application stack", not firmware. "Firmware" stays for the
  OS image a sysupgrade installs. User-facing copy follows the `bmc-ui-copy` skill.

## Commits

- **One concern per commit.** A refactor, a fix and a feature go in separate commits. A fix for an unrelated problem
  goes on its own branch.
- **No add-then-remove.** Code a later commit in the branch deletes or rewrites is folded into the commit that added it.
  Reviewers shouldn't have to read history that never merges.
- **Each commit builds and passes its tests.** Tests go in the same commit, or in their own commits directly after the
  code they cover.
- **The body says why.** It covers the problem and the reason for this approach. A subject-only commit for a non-trivial
  change is a finding.
- **Format.** The format in `.ai/instructions.md`: imperative mood, a ticket reference, lines of at most 72 characters,
  no attribution trailers. A `WIP` subject is fine in review and gone by merge, like a pin to unmerged work.
- **MR description matches the diff,** once the MR exists. Every claim in it is in the code, and every visible behaviour
  change in the code is in it.
- **A thread is addressed by the code.** On a branch with reviewer threads, check each thread answered as done against
  the code at HEAD. A reply saying it is fixed doesn't settle it.
