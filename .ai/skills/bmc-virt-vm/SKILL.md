---
name: bmc-virt-vm
description: Use when interacting with an already-running bmc-virt development VM — SSH'ing in, copying files in/out, pulling logs, or verifying kernel patches. Triggers on phrases like "ssh into the VM", "scp to the VM", "fetch the VM logs", "what does bmc.log say", or "check kernel patches still apply". The VM lifecycle itself (start/stop/clean) is user-owned — do not invoke those scripts; ask the user instead.
---

# bmc-virt VM operations

Wrapper procedures for the development VM that lives under `bmc-virt/`. Always go through the helper scripts in
`bmc-virt/scripts/` — they handle auth, port forwarding, host-key churn, and platform prerequisites for you. Do **not**
hand-roll `ssh`, `scp`, `sshpass`, `qemu-system-*`, or `ssh-keygen` invocations.

## Lifecycle ownership

The VM is a hungry, long-lived resource (QEMU + custom kernel + cross-compile builds). The user owns its lifecycle.
**Never** run `run.sh`, `stop.sh`, `clean.sh`, or `display.sh` yourself — those are user-only commands. If you need the
VM in a particular state, ask. If a cheap operation fails because the VM isn't running, surface that as a request:

> "VM doesn't appear to be running — `ssh.sh` exited with connection refused. Please start it
> (`bmc-virt/scripts/run.sh`) when you're ready and I'll continue."

This applies even when the user previously asked you to do something the VM is needed for. Resource control stays with
the user.

## Why use the scripts (for the operations you can run)

- VM listens on `localhost:2222` with hardcoded `root`/`root` credentials, accepted via `sshpass` inside the wrappers —
  manual SSH commands need the same flags every time and break silently on host-key changes.
- `vm-data/known_hosts` is project-local so VM rebuilds (which rotate the host key) don't poison `~/.ssh/known_hosts`.
- Hand-rolled commands are the most common reason a VM session drops into a "why won't it connect" loop.

## User-owned commands (ask, do not run)

| Goal                                | Command                       | Why user-owned                                                       |
| ----------------------------------- | ----------------------------- | -------------------------------------------------------------------- |
| Start VM (build + deploy + connect) | `bmc-virt/scripts/run.sh`     | Heavy: cross-compile builds, QEMU lifetime, persistent CPU/RAM cost. |
| Stop VM                             | `bmc-virt/scripts/stop.sh`    | The user may want it running for parallel work.                      |
| Wipe runtime state                  | `bmc-virt/scripts/clean.sh`   | Destructive — `git clean -fdx vm-data/`.                             |
| Open display window                 | `bmc-virt/scripts/display.sh` | Grabs a display surface on the user's machine.                       |

When suggesting these to the user, mention any flags they should consider — e.g. `run.sh --rr` (x86_64-only time-travel
debugger), `run.sh --config <name>` (deploy `data/configs/<name>.json`), `run.sh --profile <name>` (override
auto-detected build profile), `run.sh --host-path <dirs>` (extend VM's `PATH`).

## Agent-runnable (cheap, operates against the running VM)

### SSH into the VM

```
bmc-virt/scripts/ssh.sh                    # interactive login shell
bmc-virt/scripts/ssh.sh <command> [args]   # one-shot remote command
```

The interactive form runs `bash -l` on the VM. The one-shot form passes arguments through to `ssh`'s remote command.
Stdout/stderr stream straight back; exit code propagates. Prefer the one-shot form for diagnostics — it composes with
the rest of your investigation.

### Copy files

```
bmc-virt/scripts/scp.sh <local-path> root@localhost:<remote-path>
bmc-virt/scripts/scp.sh root@localhost:<remote-path> <local-path>
```

Same flag passthrough as `scp`. Forwards to port 2222 with the project's `known_hosts`.

### Hot push to a running VM

When the user says "push `<X>` to the running VM", do **not** reach for `run.sh`. `run.sh` always calls `stop.sh` first
and boots a fresh VM — that's a restart, not a push. The hot path is surgical:

1. Build the artifact natively (e.g. `nix build .#widgets-<arch>`).
2. `bmc-virt/scripts/scp.sh <built-binary> root@localhost:<remote-path>` — copy only what changed.
3. `bmc-virt/scripts/ssh.sh killall bmc-openwrt` (or the relevant service) — procd respawns it with the new binary.

Investigate guest paths first — `/etc/bmc-virt/paths.env` and similar may point at a 9p-mounted read-only store path, in
which case the target needs redirecting to a writable location before the scp lands. Never wrap the hot push in a new
`just` target, a new `run.sh --xxx` flag, or any other recipe — it stays as the three steps above. Inventing a wrapper
is wrong twice: it tends to restart the VM, and it adds code the user never asked for.

### Pull logs

```
bmc-virt/scripts/get-logs.sh
```

Pulls `bmc.log`, `relay.log`, `logread` (syslog), and `dmesg` from the VM into `bmc-virt/vm-data/logs/`, strips ANSI
escapes, and prints a sized summary. Re-running overwrites the previous capture — copy out anything you want to keep
before re-running. Log paths on the VM are read from `/etc/bmc-virt/paths.env`.

### Verify kernel patches

```
bmc-virt/scripts/check-patches.sh
```

Dry-run-applies every patch in `bmc-virt/kernel-patches/` against the upstream kernel source plus OpenWrt's
backport/pending/hack patches, without compiling anything. First run downloads the kernel source (~130 MB cached in
`/tmp/bmc-virt-patch-check/`); subsequent runs finish in seconds. Run after rebasing or when you've touched a patch —
failures here predict full-build failures.

## Common workflows

### "What is the VM doing right now?"

1. `bmc-virt/scripts/get-logs.sh`
2. `cat bmc-virt/vm-data/logs/bmc.log` (or `relay.log` / `syslog.log` / `dmesg.log`)

### "Run a quick diagnostic on the VM"

```
bmc-virt/scripts/ssh.sh /etc/init.d/b-bmc-openwrt status
bmc-virt/scripts/ssh.sh "ubus call service list '{\"name\":\"b-bmc-openwrt\"}'"
bmc-virt/scripts/ssh.sh 'cat /etc/bmc-virt/paths.env'
bmc-virt/scripts/ssh.sh 'logread -e bmc'
```

### "VM not responding"

If `ssh.sh` returns a connection error, do not attempt to start, restart, or clean the VM. Report the failure to the
user and wait. Their preferred remediation may be `run.sh`, `clean.sh && run.sh`, or "leave it alone, I'm debugging the
host."

## When extending the tooling

The scripts under `bmc-virt/scripts/` are designed for non-interactive, agent-driven use. New scripts and modifications
to existing ones must hold the same line:

- **No interactive prompts.** If the script needs input, take it as a flag. A GUI popup (TigerVNC window, dialog box,
  anything requiring a human click) blocks automation outright and is banned.
- **No timing-based waits.** `sleep N` to "let it settle" is unreliable — compile/network/IO times have inherent
  entropy. Use explicit readiness signals: marker files, socket probes, log-tail predicates.
- **Every interactive mode needs a non-interactive flag.** `--rr` (time-travel debugger), `--config <name>`,
  `--profile <name>`, etc. must work end-to-end without a user at the keyboard.

### Carve-out for explicit manual gates

A *deterministic* "tap Enter to continue" gate is acceptable when the underlying capability is genuinely missing and
building it would yak-shave the delivery — e.g. tap-injection into the compositor while the proper Wayland path is still
being built (BDK-355 precedent, 2026-04-28). The carve-out is narrow:

1. The gate is a discrete user action ("tap +D20"), not a vague "look around and continue".
2. Snapshots and measurements are tied to the gate boundaries, not to wall-clock time.
3. The gate hard-fails on non-TTY so non-interactive runs surface the missing input loudly.

Flaky waits and GUI popups remain banned even under this carve-out.

## Where state lives

- `bmc-virt/vm-data/` — runtime state (`known_hosts`, overlay disk image, cached build hashes, logs/). Wiped by
  `clean.sh`. Override the location with `BMC_VIRT_DATA=/some/path`.
- `bmc-virt/vm-data/logs/` — log capture output from `get-logs.sh`.
- `~/.local/share/bmc-virt/builder/` (macOS only) — persistent linux-builder VM state (keys, disk image, log). Survives
  reboots; not touched by `clean.sh`.

## Hard rules — never

- **Never start, stop, or wipe the VM yourself** (`run.sh`, `stop.sh`, `clean.sh`, `display.sh`). Ask the user; resource
  control is theirs.
- Never hand-roll `ssh`/`scp`/`sshpass` against `localhost:2222`. Use the scripts.
- Never edit `vm-data/known_hosts`, `~/.ssh/known_hosts`, or pass `-o StrictHostKeyChecking=no` manually — the scripts
  already configure host-key handling correctly.
- Never `qemu-system-*` directly.
- Never `git clean` outside `vm-data/`. `clean.sh` is scoped intentionally.
