---
name: ai-shared-layout
description: Use whenever about to create or modify agent-shared content in this repository — instructions files, skills, or any cross-tool AI configuration. Triggers on phrases like "add a new skill", "create a SKILL.md", "save this as a skill", "make this a skill", "update CLAUDE.md", "update AGENTS.md", "update the agent instructions", or when planning to write under `.claude/`, `.codex/`, or any other tool-specific agent directory.
---

# Shared AI instructions & skills layout

This repository hosts all shared agent content under one canonical directory — `.ai/` — and exposes it to each AI tool
via symlinks. The pattern lets Claude Code, Codex CLI, and any other agent read the same instructions and the same
skills without per-tool forks.

## Layout

```
.ai/
  instructions.md           canonical shared instructions
  skills/
    <skill-name>/SKILL.md   shared skills, one folder per skill
.claude/
  skills -> ../.ai/skills   symlink — never replace with a real directory
  *.local.*                 local-only files (settings, allowlist tweaks), gitignored
.codex/
  skills -> ../.ai/skills   symlink — never replace with a real directory
  *.local.*                 local-only files, gitignored
CLAUDE.md -> .ai/instructions.md
AGENTS.md -> .ai/instructions.md
```

`.gitignore` ignores only `.claude/*.local.*` and `.codex/*.local.*`. Everything else under those directories — the
symlinks themselves, anything else committed — is tracked and shared.

## When adding or modifying content

**Skills.** Every new skill goes in `.ai/skills/<name>/SKILL.md`. The tool-specific paths (`.claude/skills/<name>/`,
`.codex/skills/<name>/`) are symlinks; any agent reading them resolves to the same shared file. Writing through them
works but the canonical path to reference in commits and PRs is the `.ai/` one.

**Instructions.** Edit `.ai/instructions.md` directly. Don't treat `CLAUDE.md` or `AGENTS.md` as standalone files —
they're symlinks to it. The editor follows the link, but the path to *reference* (in commit messages, PR descriptions,
cross-references) is `.ai/instructions.md`.

**Local-only state.** Per-user or per-machine config (auth tokens, allowlist tweaks, dev settings) goes under `.claude/`
or `.codex/` with a `*.local.*` suffix so the gitignore keeps it out of the repo. Examples:
`.claude/settings.local.json`, `.codex/config.local.toml`.

## Don't break the symlinks

A few operations can silently turn a symlink into a regular file or directory:

- `mkdir -p .claude/skills/foo` followed by writing files there — works if the symlink resolves, but a broken symlink
  turns this into a real shadowing directory.
- Some editors "replace the file" on save instead of following the symlink, which severs the link.
- Pre-consolidation branches checked out onto the current tree without an explicit `git restore` afterwards.

Sanity check before adding new shared content:

```
ls -la .claude/skills .codex/skills CLAUDE.md AGENTS.md
```

All four should show as symlinks (`l` first character, `-> target` at the end). If any has become a regular file or
directory, restore it from git before continuing — don't work around the broken link by writing alongside it.

## Hard rules — never

- Never create a new skill under `.claude/skills/` or `.codex/skills/` directly. Always `.ai/skills/<name>/SKILL.md`.
- Never reference `CLAUDE.md` or `AGENTS.md` as standalone documents in commits / PRs / comments — reference
  `.ai/instructions.md` instead. They're symlinks.
- Never replace a symlink with a real file or directory. If a symlink is missing, restore it from git rather than
  writing alongside it.
- Never commit local-only state. `*.local.*` belongs under `.claude/`/`.codex/`; anything else committed under those
  directories is shared content and must be tool-neutral.
