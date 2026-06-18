---
name: ai-shared-layout
description: Use whenever about to create or modify agent-shared content in this repository — instructions files, skills, or any cross-tool AI configuration. Triggers on phrases like "add a new skill", "create a SKILL.md", "save this as a skill", "make this a skill", "update CLAUDE.md", "update AGENTS.md", "update the agent instructions", or when planning to write under `.claude/`, `.agents/`, or any other tool-specific agent directory.
---

# Shared AI instructions & skills layout

This repository hosts all shared agent content under one canonical directory — `.ai/` — and exposes it to each AI tool
via per-skill symlinks. The pattern lets Claude Code, Codex CLI, and any other agent read the same instructions and the
same skills without per-tool forks.

## Layout

```
.ai/
  instructions.md           canonical shared instructions
  skills/
    <skill-name>/SKILL.md   shared skills, one folder per skill
.claude/
  skills/                                      real directory
    <skill-name> -> ../../.ai/skills/<skill-name>   symlink, one per skill
  *.local.*                                    local-only files (settings, allowlist tweaks), gitignored
.agents/
  skills/                                      real directory
    <skill-name> -> ../../.ai/skills/<skill-name>   symlink, one per skill
  *.local.*                                    local-only files, gitignored
CLAUDE.md -> .ai/instructions.md
AGENTS.md -> .ai/instructions.md
```

`.gitignore` ignores only `.claude/*.local.*` and `.agents/*.local.*`. Everything else under those directories — the
symlinks themselves, anything else committed — is tracked and shared.

## When adding or modifying content

**Skills.** Every new skill goes in `.ai/skills/<name>/SKILL.md`. The tool-specific paths (`.claude/skills/<name>/`,
`.agents/skills/<name>/`) are symlinks; any agent reading them resolves to the same shared directory. Writing through
them works but the canonical path to reference in commits and PRs is the `.ai/` one.

After adding a new `.ai/skills/<name>/` directory, add both tool-specific symlinks before considering the skill
available to agents:

```
ln -s ../../.ai/skills/<name> .claude/skills/<name>
ln -s ../../.ai/skills/<name> .agents/skills/<name>
```

**Instructions.** Edit `.ai/instructions.md` directly. Don't treat `CLAUDE.md` or `AGENTS.md` as standalone files —
they're symlinks to it. The editor follows the link, but the path to *reference* (in commit messages, PR descriptions,
cross-references) is `.ai/instructions.md`.

**Local-only state.** Per-user or per-machine config (auth tokens, allowlist tweaks, dev settings) goes under `.claude/`
or `.agents/` with a `*.local.*` suffix so the gitignore keeps it out of the repo. Examples:
`.claude/settings.local.json`, `.agents/config.local.toml`.

## Don't break the symlinks

A few operations can silently turn a symlink into a regular file or directory:

- `mkdir -p .claude/skills/foo` followed by writing files there creates a real shadowing directory when `foo` is not
  already a symlink. Add the skill under `.ai/skills/foo/` first, then add the tool-specific symlinks.
- Some editors "replace the file" on save instead of following the per-skill symlink, which severs the link.
- Pre-consolidation branches checked out onto the current tree without an explicit `git restore` afterwards.

Sanity check before adding new shared content:

```
ls -ld .claude/skills .agents/skills
find .claude/skills .agents/skills -mindepth 1 -maxdepth 1 -type l -print
ls -la CLAUDE.md AGENTS.md
```

The `skills/` paths should show as directories, and every skill entry inside them should show as a symlink (`l` first
character, `-> target` at the end). `CLAUDE.md` and `AGENTS.md` should remain symlinks to `.ai/instructions.md`. If any
tool-specific skill entry has become a real directory, restore the symlink before continuing — don't work around the
broken link by writing alongside it.

## Hard rules — never

- Never create a new skill under `.claude/skills/` or `.agents/skills/` directly. Always `.ai/skills/<name>/SKILL.md`.
- Never reference `CLAUDE.md` or `AGENTS.md` as standalone documents in commits / PRs / comments — reference
  `.ai/instructions.md` instead. They're symlinks.
- Never replace a per-skill symlink with a real file or directory. If a symlink is missing, restore it rather than
  writing alongside it.
- Never commit local-only state. `*.local.*` belongs under `.claude/` or `.agents/`; anything else committed under those
  directories is shared content and must be tool-neutral.
